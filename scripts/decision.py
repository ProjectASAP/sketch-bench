#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import episodes_of, relative_errors

APP = "autoscaler"
DECISIONS = ("up", "down", "none")
ACTIONS = ("up", "down")


def decide(cpu, memory, rule):
    if cpu > rule["cpu_up"] or memory > rule["memory_up"]:
        return "up"
    if cpu < rule["cpu_down"] and memory < rule["memory_down"]:
        return "down"
    return "none"


def by_entity(query_rows, query, key):
    values, labels = {}, {}
    for r in query_rows:
        if r["labels"].get("query") != query:
            continue
        entity, arm = r["labels"][key], r["labels"]["arm"]
        arms = values.setdefault(entity, {"exact": {}, "approx": {}})
        if arm not in arms:
            raise ValueError(f"query {query}: arm {arm!r} is neither exact nor approx")
        if r["time"] in arms[arm]:
            raise ValueError(f"query {query}: {entity} {arm} evaluated twice at {r['time']}")
        arms[arm][r["time"]] = r["value"]
        labels[entity] = {k: v for k, v in r["labels"].items() if k != "arm"}
    if not values:
        raise ValueError(f"query {query} has no evaluations")
    return values, labels


def derive(query_rows, signal, cpu_query, memory_query, key, rule):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    cpu, cpu_labels = by_entity(query_rows, cpu_query, key)
    memory, memory_labels = by_entity(query_rows, memory_query, key)
    if set(cpu) != set(memory):
        raise ValueError(f"queries {cpu_query} and {memory_query} answer for different {key} sets")
    use_case = next(iter(cpu_labels.values()))["use_case"]
    app = {"use_case": use_case, "signal": signal, "app": APP}
    pairs = {(e, a): 0 for e in DECISIONS for a in DECISIONS}
    totals = {action: {"episodes": 0, "missed": 0, "breaks": 0, "delays": [], "false_actions": 0} for action in ACTIONS}
    last = None
    out = []
    for entity in sorted(cpu):
        times = sorted(cpu[entity]["exact"])
        for series in (cpu[entity]["approx"], memory[entity]["exact"], memory[entity]["approx"]):
            if sorted(series) != times:
                raise ValueError(f"{key} {entity}: queries or arms are not evaluated at the same times")
        last = max(last or times[-1], times[-1])
        for values, labels in ((cpu, cpu_labels), (memory, memory_labels)):
            out += relative_errors(run_id, labels[entity], times,
                                   [values[entity]["exact"][t] for t in times],
                                   [values[entity]["approx"][t] for t in times])
        decided = {arm: [decide(cpu[entity][arm][t], memory[entity][arm][t], rule) for t in times]
                   for arm in ("exact", "approx")}
        here = {**app, key: entity}
        for arm, decisions in decided.items():
            out += [row(t, "app_state", run_id, {**here, "arm": arm}, "decision", d, "state")
                    for t, d in zip(times, decisions)]
        for e, a in zip(decided["exact"], decided["approx"]):
            pairs[(e, a)] += 1
        for action in ACTIONS:
            found = episodes_of([d == action for d in decided["exact"]], [d == action for d in decided["approx"]])
            for i, metric, length in found["events"]:
                out.append(row(times[i], "fidelity_event", run_id, {**here, "action": action}, metric, length,
                               "evaluations"))
            total = totals[action]
            total["episodes"] += found["episodes"]
            total["missed"] += found["missed"]
            total["breaks"] += found["breaks"]
            total["delays"] += found["delays"]
            total["false_actions"] += sum(1 for _, metric, _ in found["events"] if metric == "false_positive")
        disagreements = sum(1 for e, a in zip(decided["exact"], decided["approx"]) if e != a)
        out.append(row(times[-1], "fidelity_summary", run_id, here, "disagreements", disagreements, "evaluations"))

    summary = [(f"exact_{e}_approx_{a}", n, "evaluations") for (e, a), n in pairs.items()]
    evaluations = sum(pairs.values())
    summary.append(("agreement", sum(n for (e, a), n in pairs.items() if e == a) / evaluations, "ratio"))
    for action, total in totals.items():
        summary += [(f"{action}_episodes", total["episodes"], "episodes"),
                    (f"{action}_missed_episodes", total["missed"], "episodes"),
                    (f"{action}_breaks", total["breaks"], "breaks"),
                    (f"{action}_false_actions", total["false_actions"], "spans")]
        if total["delays"]:
            summary.append((f"{action}_mean_onset_delay", sum(total["delays"]) / len(total["delays"]), "evaluations"))
    out += [row(last, "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
