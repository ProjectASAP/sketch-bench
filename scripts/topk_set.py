#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import annotations, cells_of, episodes_of, mean

APP = "topk_set"
CAPPED, NOT_CAPPED = "capped", "not capped"


def members(query_rows, query, key):
    out = {"exact": {}, "approx": {}}
    labels = None
    for r in query_rows:
        if r["labels"].get("query") != query:
            continue
        arm = r["labels"]["arm"]
        if arm not in out:
            raise ValueError(f"query {query}: arm {arm!r} is neither exact nor approx")
        at = out[arm].setdefault(r["time"], {})
        if r["labels"][key] in at:
            raise ValueError(f"query {query}: {key} {r['labels'][key]} twice in {arm} at {r['time']}")
        at[r["labels"][key]] = r["value"]
        labels = {k: v for k, v in r["labels"].items() if k not in ("arm", key)}
    if not out["exact"]:
        raise ValueError(f"query {query} has no evaluations")
    if set(out["exact"]) != set(out["approx"]):
        raise ValueError(f"query {query}: exact and approx are not evaluated at the same times")
    return labels, out


def derive(query_rows, signal, query, key):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    labels, sets = members(query_rows, query, key)
    times = sorted(sets["exact"])
    app = {"use_case": labels["use_case"], "signal": signal, "app": APP}
    out = []
    overlaps, intruders, identical, exact_size, approx_size = [], 0, 0, 0, 0
    for t in times:
        exact, approx = sets["exact"][t], sets["approx"][t]
        for member, count in exact.items():
            if member in approx and count:
                out.append(row(t, "low_level", run_id, {**labels, key: member}, "relative_error",
                               abs(approx[member] - count) / count, "ratio"))
        overlap = len(exact.keys() & approx.keys())
        overlaps.append(overlap)
        exact_size += len(exact)
        approx_size += len(approx)
        intruders += len(approx.keys() - exact.keys())
        identical += exact.keys() == approx.keys()
        out.append(row(t, "app_state", run_id, app, "overlap", overlap, "clients"))
        out.append(row(t, "app_state", run_id, app, "approx_only", len(approx.keys() - exact.keys()), "clients"))
        for arm, chosen in (("exact", exact), ("approx", approx)):
            ranked = sorted(chosen, key=lambda m: (-chosen[m], m))
            out.append(row(t, "app_state", run_id, {**app, "arm": arm}, "ranking", ", ".join(ranked), "clients"))

    found = {"episodes": 0, "missed": 0, "gaps": 0, "delays": [], "false_members": 0}
    pairs = {(e, a): 0 for e in (True, False) for a in (True, False)}
    disagreements = []
    everyone = set().union(*(set(sets[arm][t]) for arm in sets for t in times))
    for member in sorted(everyone):
        on = {arm: [member in sets[arm][t] for t in times] for arm in ("exact", "approx")}
        here = {**app, key: member}
        episodes = episodes_of(on["exact"], on["approx"])
        marks = annotations(len(times), cells_of(episodes["events"]))
        for n, t in enumerate(times):
            for arm in ("exact", "approx"):
                out.append(row(t, "app_state", run_id, {**here, "arm": arm}, "capped",
                               CAPPED if on[arm][n] else NOT_CAPPED, "state"))
            out.append(row(t, "app_state", run_id, here, "annotation", marks[n], "state"))
        for e, a in zip(on["exact"], on["approx"]):
            pairs[(e, a)] += 1
        for i, metric, length in episodes["events"]:
            out.append(row(times[i], "fidelity_event", run_id, here, metric, length, "evaluations"))
        found["episodes"] += episodes["episodes"]
        found["missed"] += episodes["missed"]
        found["gaps"] += episodes["gaps"]
        found["delays"] += episodes["delays"]
        found["false_members"] += sum(1 for _, metric, _ in episodes["events"] if metric == "false_positive")
        disagreements.append((sum(1 for e, a in zip(on["exact"], on["approx"]) if e != a), member))

    for count, member in sorted(disagreements, key=lambda d: (-d[0], d[1])):
        out.append(row(times[-1], "fidelity_summary", run_id, {**app, key: member}, "disagreements", count,
                       "evaluations"))
    summary = [
        ("true_positive", pairs[(True, True)], "client-evaluations"),
        ("false_positive", pairs[(False, True)], "client-evaluations"),
        ("false_negative", pairs[(True, False)], "client-evaluations"),
        ("true_negative", pairs[(False, False)], "client-evaluations"),
        ("mean_overlap", sum(overlaps) / len(overlaps), "clients"),
        ("mean_list_size", exact_size / len(times), "clients"),
        ("precision", sum(overlaps) / approx_size if approx_size else 0, "ratio"),
        ("recall", sum(overlaps) / exact_size if exact_size else 0, "ratio"),
        ("identical_sets", identical, "evaluations"),
        ("evaluations", len(times), "evaluations"),
        ("intruder_slots", intruders, "client-evaluations"),
        ("entries", found["episodes"], "episodes"),
        ("missed_entries", found["missed"], "episodes"),
        ("gaps", found["gaps"], "gaps"),
        ("false_member_spans", found["false_members"], "spans"),
    ]
    if found["delays"]:
        summary.append(("mean_onset_delay", mean(found["delays"]), "evaluations"))
    out += [row(times[-1], "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
