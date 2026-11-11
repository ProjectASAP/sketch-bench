#!/usr/bin/env python3

import math

from approx_rows import row
from bounds import PLAIN, all_of, any_of, ge, gt, hi, judge, lo, lt, point
from membership import membership_fidelity

APP = "rule"
ACTIVE, IDLE = "active", "idle"
KEY = "entity"


def all_labels(labels):
    return ".".join(v for k, v in sorted(labels.items()) if k not in ("use_case", "signal", "query", "arm"))


def collect(query_rows, queries, entity_of, widen=None):
    """values[arm][entity][query] is one value per time of `times`, None where the arm gave no answer."""
    found, times, use_case = {}, set(), None
    for r in query_rows:
        query = r["labels"].get("query")
        if query not in queries:
            continue
        arm = r["labels"]["arm"]
        if arm not in ("exact", "approx"):
            raise ValueError(f"query {query}: arm {arm!r} is neither exact nor approx")
        entity = entity_of(r["labels"])
        at = found.setdefault(arm, {}).setdefault(entity, {}).setdefault(query, {})
        if r["time"] in at:
            raise ValueError(f"query {query}: {entity} {arm} evaluated twice at {r['time']}")
        value = r["value"]
        if widen is not None and arm == "approx":
            value = widen(query, r["time"], r["labels"], value)
        at[r["time"]] = value
        times.add(r["time"])
        use_case = r["labels"]["use_case"]
    if not times:
        raise ValueError(f"queries {sorted(queries)} have no evaluations")
    times = sorted(times)
    values = {arm: {entity: {q: [by_query.get(q, {}).get(t) for t in times] for q in queries}
                    for entity, by_query in found.get(arm, {}).items()}
              for arm in ("exact", "approx")}
    return use_case, times, values


def known(values):
    return [point(v) for v in values if v is not None]


def history_band(values, i, size, minimum):
    past = known(values[max(0, i - size):i])
    if len(past) < minimum:
        return None
    centre = sum(past) / len(past)
    spread = math.sqrt(sum((v - centre) ** 2 for v in past) / len(past))
    return centre, spread


def above_band(values, size, k, minimum=None):
    """Above the mean plus k standard deviations of the previous `size` values."""
    out = []
    for i, v in enumerate(values):
        band = history_band(values, i, size, minimum or size)
        out.append(band is not None and gt(v, band[0] + k * band[1]))
    return out


def outside_band(values, size, k, minimum=None):
    """More than k standard deviations from the mean of the previous `size` values, either way."""
    out = []
    for i, v in enumerate(values):
        band = history_band(values, i, size, minimum or size)
        out.append(band is not None and any_of(gt(v, band[0] + k * band[1]), lt(v, band[0] - k * band[1])))
    return out


def above_decayed_band(values, decay, k, minimum):
    """Above a baseline whose weights decay by `decay` per evaluation, plus k decayed standard deviations."""
    out, weight, centre, square, seen = [], 0.0, 0.0, 0.0, 0
    for v in values:
        spread = math.sqrt(max(0.0, square - centre ** 2))
        out.append(seen >= minimum and gt(v, centre + k * spread))
        if v is not None:
            v = point(v)
            weight = decay * weight + 1
            centre += (v - centre) / weight
            square += (v * v - square) / weight
            seen += 1
    return out


def above_mean_times(values, size, factor, minimum=None):
    """Above `factor` times the mean of the previous `size` values."""
    out = []
    for i, v in enumerate(values):
        band = history_band(values, i, size, minimum or size)
        out.append(band is not None and gt(v, factor * band[0]))
    return out


def cusum_above(values, training, k, h):
    """One-sided CUSUM: S = max(0, S + x - mean - k * sd), with mean and sd from the first `training` values;
    holds while S > h * sd. S is capped at 2 * h * sd so that it falls back below h once the change ends.
    An interval answer carries a lower and an upper S.
    """
    band = history_band(values, training, training, training)
    if band is None:
        return [False] * len(values)
    centre, spread = band
    out, low, high = [], 0.0, 0.0
    for i, v in enumerate(values):
        if i < training or v is None:
            out.append(False)
            continue
        limit = h * spread
        low = min(2 * limit, max(0.0, low + lo(v) - centre - k * spread))
        high = min(2 * limit, max(0.0, high + hi(v) - centre - k * spread))
        out.append(True if low > limit else False if high <= limit else None)
    return out


def above(values, threshold):
    return [gt(v, threshold) for v in values]


def below(values, threshold):
    return [lt(v, threshold) for v in values]


def at_least(values, threshold):
    return [ge(v, threshold) for v in values]


def both(a, b):
    return [all_of(x, y) for x, y in zip(a, b)]


def either(a, b):
    return [any_of(x, y) for x, y in zip(a, b)]


def as_is(flags):
    return list(flags)


def for_consecutive(n):
    """Active once the condition has held for n evaluations in a row."""
    def hold(flags):
        out, run = [], 0
        for f in flags:
            run = run + 1 if f else 0
            out.append(run >= n)
        return out
    return hold


def x_of_last_y(x, y):
    """Active when the condition held at least x times in the last y evaluations."""
    return lambda flags: [sum(flags[max(0, i - y + 1):i + 1]) >= x for i in range(len(flags))]


def latch(evaluations, extend=True, while_held=False):
    """Active for `evaluations` after the condition holds.

    extend: a hit while active restarts the time. while_held: the state also outlasts its time while the
    condition still holds, and ends at the first evaluation after that where it does not.
    """
    def hold(flags):
        out, until, active = [], 0, False
        for i, f in enumerate(flags):
            if active and i >= until and not (while_held and f):
                active = False
            if f and (extend or not active):
                until, active = i + evaluations, True
            out.append(active)
        return out
    return hold


def planned_rules(rule_set, queries):
    ids = {q["id"] for q in queries}
    return {name: rule for name, rule in rule_set.items() if set(rule["queries"]) <= ids}


def rule_signals(rule_set):
    return lambda query: [name for name, rule in rule_set.items() if query in rule["queries"]]


def derive(query_rows, rules, policy=PLAIN, widen=None):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    out = []
    for name, rule in rules.items():
        out += rule_rows(run_id, name, rule, query_rows, policy, widen)
    return out


def rule_rows(run_id, name, rule, query_rows, policy=PLAIN, widen=None):
    use_case, times, values = collect(query_rows, rule["queries"], rule.get("entity", all_labels), widen)
    queries = rule["queries"]

    def check(*lists):
        return rule["condition"](dict(zip(queries, lists)))

    checks = {"exact": {entity: rule["condition"](by_query) for entity, by_query in values["exact"].items()},
              "approx": {}}
    unsure = 0
    missing = {q: [None] * len(times) for q in queries}
    for entity, by_query in values["approx"].items():
        exact = values["exact"].get(entity, missing)
        checks["approx"][entity], open_ = judge(check, [by_query[q] for q in queries], [exact[q] for q in queries],
                                                policy)
        unsure += open_
    hold = rule.get("hold", as_is)
    on = {arm: {entity: hold(flags) for entity, flags in by_entity.items()} for arm, by_entity in checks.items()}
    app = {"use_case": use_case, "signal": name, "app": APP}
    out, found = membership_fidelity(run_id, app, KEY, times, on, "state", (ACTIVE, IDLE))
    total = found["true_positive"] + found["false_positive"] + found["false_negative"] + found["true_negative"]
    held = found["true_positive"] + found["false_positive"] + found["false_negative"]
    summary = [(metric, found[metric], "entity-evaluations")
               for metric in ("true_positive", "false_positive", "false_negative", "true_negative")]
    summary += [("agreement", (total - found["false_positive"] - found["false_negative"]) / total, "ratio"),
                ("active_agreement", found["true_positive"] / held if held else 1.0, "ratio"),
                ("entities", len(set(on["exact"]) | set(on["approx"])), "entities"),
                ("entries", found["entries"], "episodes"),
                ("missed_entries", found["missed_entries"], "episodes"),
                ("gaps", found["gaps"], "gaps"),
                ("false_spans", found["false_spans"], "spans")]
    if "mean_onset_delay" in found:
        summary.append(("mean_onset_delay", found["mean_onset_delay"], "evaluations"))
    if policy != PLAIN:
        summary.append(("unsure_checks", unsure, "entity-evaluations"))
    out += [row(times[-1], "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
