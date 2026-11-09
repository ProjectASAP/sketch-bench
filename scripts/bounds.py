#!/usr/bin/env python3

import math
import re

PLAIN, BOUND, RERUN = "plain", "bound", "rerun"
POLICIES = (PLAIN, BOUND, RERUN)
LOWER, UPPER = "_lower_bound", "_upper_bound"
QUANTILE = re.compile(r"quantile_over_time\(\s*([0-9.]+)\s*,")
NOT_SERIES = ("use_case", "signal", "query", "arm")


class Bounded:
    """An approximate answer and the interval its error bound puts the exact answer in."""

    def __init__(self, value, lo, hi):
        self.value, self.lo, self.hi = value, min(lo, hi), max(lo, hi)

    def __repr__(self):
        return f"Bounded({self.value}, {self.lo}, {self.hi})"


def lo(x):
    return x.lo if isinstance(x, Bounded) else x


def hi(x):
    return x.hi if isinstance(x, Bounded) else x


def point(x):
    return x.value if isinstance(x, Bounded) else x


def gt(x, threshold):
    """True when all of x is above the threshold, False when none of it is, None when unsure."""
    if x is None:
        return False
    if lo(x) > threshold:
        return True
    if hi(x) <= threshold:
        return False
    return None


def ge(x, threshold):
    if x is None:
        return False
    if lo(x) >= threshold:
        return True
    if hi(x) < threshold:
        return False
    return None


def lt(x, threshold):
    if x is None:
        return False
    if hi(x) < threshold:
        return True
    if lo(x) >= threshold:
        return False
    return None


def ratio(x, y):
    if x is None or y is None:
        return None
    if not isinstance(x, Bounded) and not isinstance(y, Bounded):
        return x / y
    if lo(y) <= 0 <= hi(y):
        return Bounded(point(x) / point(y) if point(y) else math.inf, -math.inf, math.inf)
    corners = [a / b for a in (lo(x), hi(x)) for b in (lo(y), hi(y))]
    return Bounded(point(x) / point(y), min(corners), max(corners))


def any_of(*flags):
    if any(f is True for f in flags):
        return True
    return None if any(f is None for f in flags) else False


def all_of(*flags):
    if any(f is False for f in flags):
        return False
    return None if any(f is None for f in flags) else True


def settle(checks, exact_checks, policy, default=False):
    """Turn unsure checks (None) into decisions: `default` under BOUND, the exact plan's check under RERUN."""
    out, unsure = [], 0
    for c, e in zip(checks, exact_checks):
        if c is None:
            unsure += 1
            out.append(e if policy == RERUN else default)
        else:
            out.append(c)
    return out, unsure


def judge(check, approx_args, exact_args, policy, default=False):
    """Run check(*approx_args) and decide its unsure results by the policy.

    Under RERUN an unsure evaluation reruns the check's queries with the exact plan: its values become the
    exact ones and the check runs again, so a check that keeps history sees the rerun answers.
    A check still unsure after its rerun, because history from earlier interval answers keeps it open,
    takes the exact plan's check. Returns the checks and how many evaluations were unsure.
    """
    if policy != RERUN:
        return settle(check(*approx_args), check(*exact_args), policy, default)
    values = [list(v) for v in approx_args]
    rerun = set()
    while True:
        out = check(*values)
        open_ = [i for i, c in enumerate(out) if c is None]
        fresh = [i for i in open_ if i not in rerun]
        if not fresh:
            if open_:
                exact = check(*exact_args)
                out = [exact[i] if c is None else c for i, c in enumerate(out)]
            return out, len(rerun)
        for i in fresh:
            rerun.add(i)
            for v, e in zip(values, exact_args):
                v[i] = e[i]


def series_key(labels):
    return tuple(sorted((k, v) for k, v in labels.items() if k not in NOT_SERIES and k != "__name__"))


def with_bound_queries(spec, max_error):
    """The spec plus, for each quantile query, the same query at phi - max_error and phi + max_error."""
    extra = []
    for q in spec["queries"]:
        found = QUANTILE.search(q["promql"])
        if not found:
            continue
        phi = float(found.group(1))
        for suffix, shifted in ((LOWER, max(0.0, phi - max_error)), (UPPER, min(1.0, phi + max_error))):
            promql = q["promql"][:found.start(1)] + f"{shifted:g}" + q["promql"][found.end(1):]
            extra.append({**q, "id": q["id"] + suffix, "promql": promql})
    return {**spec, "queries": spec["queries"] + extra}


def is_bound_query(query_id):
    return query_id.endswith(LOWER) or query_id.endswith(UPPER)


def guaranteed_metric(plan):
    """The error metric a plan guarantees, or None for an exact plan or one without a guarantee."""
    metrics = {(readout.get("error") or {}).get("metric") for analytics in plan.get("sketch_analytics", [])
               for readout in analytics.get("readouts") or [{}]}
    if not plan.get("sketches") or None in metrics or not metrics:
        return None
    return metrics.pop() if len(metrics) == 1 else None


class Widen:
    """Puts each approximate answer in the interval its plan's error bound gives at the arm's target.

    Rank bounds read the same query run at phi -/+ the target; relative and absolute bounds scale the answer.
    Plans without a guarantee, and bounds that need the stream's norm, keep the bare answer.
    """

    def __init__(self, approx, max_error):
        self.max_error = max_error
        self.metric, self.companions = {}, {}
        by_id = {q["id"]: q for q in approx["queries"]}
        for q in approx["queries"]:
            if is_bound_query(q["id"]) or "plan" not in q:
                continue
            metric = guaranteed_metric(q["plan"])
            if metric == "rank":
                ends = [by_id.get(q["id"] + suffix) for suffix in (LOWER, UPPER)]
                if any(e is None or "answers" not in e for e in ends):
                    continue
                self.companions[q["id"]] = [{(a["time"], series_key(a["series"])): a["value"] for a in e["answers"]}
                                            for e in ends]
            if metric in ("rank", "relative", "absolute"):
                self.metric[q["id"]] = metric

    def bounded_queries(self):
        return sorted(self.metric)

    def __call__(self, query, time, labels, value):
        metric = self.metric.get(query)
        if value is None or metric is None:
            return value
        if metric == "relative":
            return Bounded(value, value / (1 + self.max_error),
                           value / (1 - self.max_error) if self.max_error < 1 else float("inf"))
        if metric == "absolute":
            return Bounded(value, value - self.max_error, value + self.max_error)
        key = (time, series_key(labels))
        low, high = (c.get(key) for c in self.companions[query])
        if low is None or high is None:
            return value
        return Bounded(value, min(low, value), max(high, value))


def widen_values(widen, query, times, labels, values):
    if widen is None:
        return values
    return [widen(query, t, labels, v) for t, v in zip(times, values)]
