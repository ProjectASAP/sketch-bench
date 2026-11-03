#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import (EVENT_LABELS, NOTHING, annotations, cells_of, delay_summary, episodes_of,
                             relative_errors, series)

APP = "rollout_controller"
ROLLBACK, PROMOTE = "rollback", "promote"
SLOWER, NOT_SLOWER = "slower", "not slower"
CHECKS, STREAK = 10, 3


def decisions_of(slower, checks=CHECKS, streak=STREAK):
    out = []
    for first in range(0, len(slower) - checks + 1, checks):
        run = 0
        for i in range(first, first + checks):
            run = run + 1 if slower[i] else 0
            if run == streak:
                out.append((i, ROLLBACK))
                break
        else:
            out.append((first + checks - 1, PROMOTE))
    return out


def outcome(exact, approx):
    if exact == ROLLBACK:
        return "true_positive" if approx == ROLLBACK else "false_negative"
    return "false_positive" if approx == ROLLBACK else "true_negative"


def rollouts(exact_slower, approx_slower, checks=CHECKS, streak=STREAK):
    found = episodes_of(exact_slower, approx_slower)
    gaps = [(i, length) for i, metric, length in found["events"] if metric == "gap"]
    cells = cells_of(found["events"], ("gap",))
    decided = []
    for n, ((i, exact), (j, approx)) in enumerate(zip(decisions_of(exact_slower, checks, streak),
                                                     decisions_of(approx_slower, checks, streak))):
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
    return {"rollouts": decided, "gaps": gaps, "annotations": annotations(len(exact_slower), cells)}


def actions(length, decided, arm):
    out = [NOTHING] * length
    for rollout in decided:
        i, decision = rollout[arm]
        out[i] = decision
    return out


def derive(query_rows, signal, canary, stable, factor):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    canary_labels, _, times, canary_exact, canary_approx = series(query_rows, canary)
    stable_labels, _, stable_times, stable_exact, stable_approx = series(query_rows, stable)
    if times != stable_times:
        raise ValueError(f"queries {canary} and {stable} are not evaluated at the same times")
    out = relative_errors(run_id, canary_labels, times, canary_exact, canary_approx)
    out += relative_errors(run_id, stable_labels, times, stable_exact, stable_approx)
    app = {"use_case": canary_labels["use_case"], "signal": signal, "app": APP}
    ratio = {"exact": [c / s for c, s in zip(canary_exact, stable_exact)],
             "approx": [c / s for c, s in zip(canary_approx, stable_approx)]}
    slower = {arm: [r > factor for r in ratios] for arm, ratios in ratio.items()}
    found = rollouts(slower["exact"], slower["approx"])
    decided = found["rollouts"]
    acted = {arm: actions(len(times), decided, arm) for arm in ("exact", "approx")}
    for n, t in enumerate(times):
        for arm in ("exact", "approx"):
            here = {**app, "arm": arm}
            out.append(row(t, "app_state", run_id, here, "ratio", ratio[arm][n], "ratio"))
            out.append(row(t, "app_state", run_id, here, "slower", SLOWER if slower[arm][n] else NOT_SLOWER, "state"))
            out.append(row(t, "app_state", run_id, here, "action", acted[arm][n], "state"))
        out.append(row(t, "app_state", run_id, app, "tolerance", factor, "ratio"))
        out.append(row(t, "app_state", run_id, app, "annotation", found["annotations"][n], "state"))

    for i, length in found["gaps"]:
        out.append(row(times[i], "fidelity_event", run_id, app, "gap", length, "evaluations"))
    for rollout in decided:
        (i, exact), (j, approx) = rollout["exact"], rollout["approx"]
        labels = {**app, "rollout": str(rollout["rollout"]), "exact": exact, "approx": approx}
        out.append(row(times[rollout["first"]], "fidelity_event", run_id, labels, "rollout", rollout["outcome"],
                       "outcome"))
        if rollout["delay"]:
            out.append(row(times[j], "fidelity_event", run_id, app, "decision_delay", rollout["delay"],
                           "evaluations"))

    outcomes = [rollout["outcome"] for rollout in decided]
    summary = [(metric, outcomes.count(metric), "rollouts")
               for metric in ("true_positive", "false_positive", "false_negative", "true_negative")]
    summary += [("rollouts", len(decided), "rollouts"),
                ("agreement", (outcomes.count("true_positive") + outcomes.count("true_negative")) / len(decided)
                 if decided else 1.0, "ratio"),
                ("gaps", len(found["gaps"]), "gaps")]
    summary += delay_summary([rollout["delay"] for rollout in decided if rollout["outcome"] == "true_positive"],
                             "rollback_delay", "rollbacks", "early_rollbacks", "late_rollbacks")
    out += [row(times[-1], "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
