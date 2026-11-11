#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import annotations, cells_of, episodes_of, mean


def membership_fidelity(run_id, app, key, times, on, state, states):
    """Compare, entity by entity, when the exact and the approx arm hold an entity in a state.

    on[arm][entity] is one boolean per evaluation time; states names the two values of the state row.
    Returns the rows and the counts summed over entities.
    """
    out = []
    found = {"episodes": 0, "missed": 0, "gaps": 0, "delays": [], "false_spans": 0}
    pairs = {(e, a): 0 for e in (True, False) for a in (True, False)}
    disagreements = []
    for entity in sorted(set(on["exact"]) | set(on["approx"])):
        held = {arm: on[arm].get(entity, [False] * len(times)) for arm in ("exact", "approx")}
        here = {**app, key: entity}
        episodes = episodes_of(held["exact"], held["approx"])
        marks = annotations(len(times), cells_of(episodes["events"]))
        for n, t in enumerate(times):
            for arm in ("exact", "approx"):
                out.append(row(t, "app_state", run_id, {**here, "arm": arm}, state,
                               states[0] if held[arm][n] else states[1], "state"))
            out.append(row(t, "app_state", run_id, here, "annotation", marks[n], "state"))
        for e, a in zip(held["exact"], held["approx"]):
            pairs[(e, a)] += 1
        for i, metric, length in episodes["events"]:
            out.append(row(times[i], "fidelity_event", run_id, here, metric, length, "evaluations"))
        found["episodes"] += episodes["episodes"]
        found["missed"] += episodes["missed"]
        found["gaps"] += episodes["gaps"]
        found["delays"] += episodes["delays"]
        found["false_spans"] += sum(1 for _, metric, _ in episodes["events"] if metric == "false_positive")
        disagreements.append((sum(1 for e, a in zip(held["exact"], held["approx"]) if e != a), entity))
    for count, entity in sorted(disagreements, key=lambda d: (-d[0], d[1])):
        out.append(row(times[-1], "fidelity_summary", run_id, {**app, key: entity}, "disagreements", count,
                       "evaluations"))
    counts = {"true_positive": pairs[(True, True)], "false_positive": pairs[(False, True)],
              "false_negative": pairs[(True, False)], "true_negative": pairs[(False, False)],
              "entries": found["episodes"], "missed_entries": found["missed"], "gaps": found["gaps"],
              "false_spans": found["false_spans"]}
    if found["delays"]:
        counts["mean_onset_delay"] = mean(found["delays"])
    return out, counts
