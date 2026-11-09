#!/usr/bin/env python3

from approx_rows import row
from bounds import PLAIN, any_of, gt, lt, point, ratio, settle, widen_values
from threshold_alert import EVENT_LABELS, NOTHING, annotations, cells_of, delay_summary, episodes_of, series

APP = "rollout_controller"
ROLLBACK, PROMOTE = "rollback", "promote"
FAILED, PASSED = "failed", "passed"
CHECKS, STREAK, TOLERANCE = 10, 3, 1.1
MAX_P99 = 0.5
FLAGGER_MIN_SUCCESS_RATE, FAILURE_THRESHOLD, PASSES_TO_PROMOTE = 0.99, 10, 25
ARGO_ROLLOUTS_MIN_SUCCESS_RATE, ARGO_ROLLOUTS_INTERVAL, ARGO_ROLLOUTS_COUNT, FAILURE_LIMIT = 0.95, 5, 10, 3
CLOUDWATCH_CHECKS, BREACHING_POINTS, EVALUATED_POINTS = 10, 4, 5


def slower(p99_canary, p99_stable):
    return [gt(ratio(c, s), TOLERANCE) for c, s in zip(p99_canary, p99_stable)]


def flagger_failed(p99_canary, success_rate_canary):
    return [any_of(gt(p, MAX_P99), lt(s, FLAGGER_MIN_SUCCESS_RATE))
            for p, s in zip(p99_canary, success_rate_canary)]


def argo_rollouts_failed(success_rate_canary):
    return [lt(s, ARGO_ROLLOUTS_MIN_SUCCESS_RATE) for s in success_rate_canary]


def cloudwatch_breaching(p99_canary):
    return [gt(p, MAX_P99) for p in p99_canary]


def in_a_row(failed):
    run = 0
    for i, f in enumerate(failed):
        run = run + 1 if f else 0
        if run == STREAK:
            return i, ROLLBACK
    return len(failed) - 1, PROMOTE


def failures_or_passes(failed):
    failures = passes = 0
    for i, f in enumerate(failed):
        failures += f
        passes += not f
        if failures == FAILURE_THRESHOLD:
            return i, ROLLBACK
        if passes == PASSES_TO_PROMOTE:
            return i, PROMOTE
    return len(failed) - 1, PROMOTE


def failures_over_limit(failed):
    failures = 0
    for i, f in enumerate(failed):
        failures += f
        if failures > FAILURE_LIMIT:
            return i, ROLLBACK
    return len(failed) - 1, PROMOTE


def m_of_last_n(failed):
    for i in range(len(failed)):
        if sum(failed[max(0, i - EVALUATED_POINTS + 1):i + 1]) >= BREACHING_POINTS:
            return i, ROLLBACK
    return len(failed) - 1, PROMOTE


RULES = {
    "ours": {"queries": ("p99_canary", "p99_stable"), "check": slower, "every": 1, "checks": CHECKS,
             "decide": in_a_row},
    "flagger": {"queries": ("p99_canary", "success_rate_canary"), "check": flagger_failed, "every": 1,
                "checks": FAILURE_THRESHOLD - 1 + PASSES_TO_PROMOTE, "decide": failures_or_passes},
    "argo_rollouts": {"queries": ("success_rate_canary",), "check": argo_rollouts_failed,
                      "every": ARGO_ROLLOUTS_INTERVAL, "checks": ARGO_ROLLOUTS_COUNT, "decide": failures_over_limit},
    "cloudwatch": {"queries": ("p99_canary",), "check": cloudwatch_breaching, "every": 1,
                   "checks": CLOUDWATCH_CHECKS, "decide": m_of_last_n},
}


def decisions_of(failed, checks=CHECKS, decide=in_a_row):
    out = []
    for first in range(0, len(failed) - checks + 1, checks):
        i, decision = decide(failed[first:first + checks])
        out.append((first + i, decision))
    return out


def outcome(exact, approx):
    if exact == ROLLBACK:
        return "true_positive" if approx == ROLLBACK else "false_negative"
    return "false_positive" if approx == ROLLBACK else "true_negative"


def rollouts(exact_failed, approx_failed, checks=CHECKS, decide=in_a_row):
    found = episodes_of(exact_failed, approx_failed)
    gaps = [(i, length) for i, metric, length in found["events"] if metric == "gap"]
    cells = cells_of(found["events"], ("gap",))
    decided = []
    for n, ((i, exact), (j, approx)) in enumerate(zip(decisions_of(exact_failed, checks, decide),
                                                     decisions_of(approx_failed, checks, decide))):
        result = outcome(exact, approx)
        delay = j - i if exact == approx else None
        if delay:
            cells.append((j, j + 1, EVENT_LABELS["decision_delay"]))
        if result == "false_positive":
            cells.append((j, j + 1, EVENT_LABELS["false_positive"]))
        if result == "false_negative":
            cells.append((i, i + 1, EVENT_LABELS["false_negative"]))
        decided.append({"rollout": n + 1, "first": n * checks, "exact": (i, exact), "approx": (j, approx),
                        "outcome": result, "delay": delay})
    return {"rollouts": decided, "gaps": gaps, "annotations": annotations(len(exact_failed), cells)}


def actions(length, decided, arm):
    out = [NOTHING] * length
    for rollout in decided:
        i, decision = rollout[arm]
        out[i] = decision
    return out


def derive(query_rows, rules=RULES, policy=PLAIN, widen=None):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    queries = sorted({q for rule in rules.values() for q in rule["queries"]})
    found = {q: series(query_rows, q) for q in queries}
    labels, _, times, _, _ = found[queries[0]]
    out = []
    values = {"exact": {}, "approx": {}}
    for q in queries:
        q_labels, _, q_times, exact, approx = found[q]
        if q_times != times:
            raise ValueError(f"queries {queries[0]} and {q} are not evaluated at the same times")
        values["exact"][q], values["approx"][q] = exact, widen_values(widen, q, times, q_labels, approx)
    app = {"use_case": labels["use_case"], "app": APP}
    for n, t in enumerate(times):
        for arm, by_query in values.items():
            out.append(row(t, "app_state", run_id, {**app, "arm": arm}, "ratio",
                           point(by_query["p99_canary"][n]) / point(by_query["p99_stable"][n]), "ratio"))
        out.append(row(t, "app_state", run_id, app, "tolerance", TOLERANCE, "ratio"))
    for name, rule in rules.items():
        out += rule_rows(run_id, {**app, "signal": name}, rule, times, values, policy)
    return out


def rule_rows(run_id, app, rule, times, values, policy=PLAIN):
    every = rule["every"]
    checked = times[::every]
    failed = {arm: rule["check"](*[values[arm][q] for q in rule["queries"]])[::every] for arm in values}
    failed["approx"], unsure = settle(failed["approx"], failed["exact"], policy)
    found = rollouts(failed["exact"], failed["approx"], rule["checks"], rule["decide"])
    decided = found["rollouts"]
    acted = {arm: actions(len(checked), decided, arm) for arm in values}
    out = []
    for n, t in enumerate(checked):
        for arm in values:
            here = {**app, "arm": arm}
            out.append(row(t, "app_state", run_id, here, "check", FAILED if failed[arm][n] else PASSED, "state"))
            out.append(row(t, "app_state", run_id, here, "action", acted[arm][n], "state"))
        out.append(row(t, "app_state", run_id, app, "annotation", found["annotations"][n], "state"))

    for i, length in found["gaps"]:
        out.append(row(checked[i], "fidelity_event", run_id, app, "gap", length * every, "evaluations"))
    for rollout in decided:
        (i, exact), (j, approx) = rollout["exact"], rollout["approx"]
        labels = {**app, "rollout": str(rollout["rollout"]), "exact": exact, "approx": approx}
        out.append(row(checked[rollout["first"]], "fidelity_event", run_id, labels, "rollout", rollout["outcome"],
                       "outcome"))
        if rollout["delay"]:
            out.append(row(checked[j], "fidelity_event", run_id, app, "decision_delay", rollout["delay"] * every,
                           "evaluations"))

    outcomes = [rollout["outcome"] for rollout in decided]
    summary = [(metric, outcomes.count(metric), "rollouts")
               for metric in ("true_positive", "false_positive", "false_negative", "true_negative")]
    summary += [("rollouts", len(decided), "rollouts"),
                ("agreement", (outcomes.count("true_positive") + outcomes.count("true_negative")) / len(decided)
                 if decided else 1.0, "ratio"),
                ("gaps", len(found["gaps"]), "gaps")]
    if policy != PLAIN:
        summary.append(("unsure_checks", unsure, "checks"))
    summary += delay_summary([rollout["delay"] * every for rollout in decided
                              if rollout["outcome"] == "true_positive"],
                             "rollback_delay", "rollbacks", "early_rollbacks", "late_rollbacks")
    out += [row(checked[-1], "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
