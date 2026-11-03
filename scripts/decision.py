#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import EVENT_LABELS, NOTHING, annotations, delay_summary, relative_errors

APP = "autoscaler"
DECISIONS = ("up", "down", NOTHING)
ACTIONS = ("up", "down")
MATCH_WINDOW = 2


def decide(cpu, memory, rule):
    if cpu > rule["cpu_up"] or memory > rule["memory_up"]:
        return "up"
    if cpu < rule["cpu_down"] and memory < rule["memory_down"]:
        return "down"
    return NOTHING


def actions_of(decisions):
    return [d if i > 0 and d != NOTHING and decisions[i - 1] != d else NOTHING for i, d in enumerate(decisions)]


def match_actions(exact, approx, window=MATCH_WINDOW):
    candidates = sorted((abs(j - i), j, i) for i, action in enumerate(exact) if action != NOTHING
                        for j in range(max(0, i - window), min(len(approx), i + window + 1))
                        if approx[j] == action)
    matched, taken = {}, set()
    for _, j, i in candidates:
        if i not in matched and j not in taken:
            matched[i] = j
            taken.add(j)
    missed = [i for i, a in enumerate(exact) if a != NOTHING and i not in matched]
    false = [j for j, a in enumerate(approx) if a != NOTHING and j not in taken]
    return dict(sorted(matched.items())), missed, false


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
    totals = {action: {"true_positive": 0, "false_positive": 0, "false_negative": 0, "delays": []}
              for action in ACTIONS}
    true_negative = 0
    last = None
    disagreements = []
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
        acted = {arm: actions_of(decisions) for arm, decisions in decided.items()}
        here = {**app, key: entity}
        for arm in ("exact", "approx"):
            out += [row(t, "app_state", run_id, {**here, "arm": arm}, "decision", d, "state")
                    for t, d in zip(times, decided[arm])]
            out += [row(t, "app_state", run_id, {**here, "arm": arm}, "action", a, "state")
                    for t, a in zip(times, acted[arm])]
        for e, a in zip(decided["exact"], decided["approx"]):
            pairs[(e, a)] += 1

        exact, approx = acted["exact"], acted["approx"]
        matched, missed, false = match_actions(exact, approx)
        cells = []
        for i, j in matched.items():
            totals[exact[i]]["true_positive"] += 1
            totals[exact[i]]["delays"].append(j - i)
            if j != i:
                cells.append((j, j + 1, EVENT_LABELS["onset_delay"]))
                out.append(row(times[j], "fidelity_event", run_id, {**here, "action": exact[i]}, "onset_delay",
                               j - i, "evaluations"))
        for i in missed:
            totals[exact[i]]["false_negative"] += 1
            cells.append((i, i + 1, EVENT_LABELS["false_negative"]))
            out.append(row(times[i], "fidelity_event", run_id, {**here, "action": exact[i]}, "false_negative", 1,
                           "evaluations"))
        for j in false:
            totals[approx[j]]["false_positive"] += 1
            cells.append((j, j + 1, EVENT_LABELS["false_positive"]))
            out.append(row(times[j], "fidelity_event", run_id, {**here, "action": approx[j]}, "false_positive", 1,
                           "evaluations"))
        out += [row(t, "app_state", run_id, here, "annotation", mark, "state")
                for t, mark in zip(times, annotations(len(times), cells))]
        true_negative += sum(1 for e, a in zip(exact, approx) if e == NOTHING and a == NOTHING)
        disagreements.append((sum(1 for e, a in zip(exact, approx) if e != a), entity, times[-1]))

    for count, entity, t in sorted(disagreements, key=lambda d: (-d[0], d[1])):
        out.append(row(t, "fidelity_summary", run_id, {**app, key: entity}, "disagreements", count, "evaluations"))
    summary = [(f"exact_{e}_approx_{a}", n, "evaluations") for (e, a), n in pairs.items()]
    evaluations = sum(pairs.values())
    summary.append(("decision_agreement", sum(n for (e, a), n in pairs.items() if e == a) / evaluations, "ratio"))
    counts = {metric: sum(total[metric] for total in totals.values())
              for metric in ("true_positive", "false_positive", "false_negative")}
    summary += [(metric, n, "actions") for metric, n in counts.items()]
    summary.append(("true_negative", true_negative, "evaluations"))
    actions = sum(counts.values())
    summary.append(("action_agreement", counts["true_positive"] / actions if actions else 1.0, "ratio"))
    summary += delay_summary([d for total in totals.values() for d in total["delays"]], "onset_delay", "actions",
                             "early_actions", "late_actions")
    for action, total in totals.items():
        summary += [(f"{action}_{metric}", total[metric], "actions")
                    for metric in ("true_positive", "false_positive", "false_negative")]
        summary += [(f"{action}_{metric}", value, unit) for metric, value, unit in
                    delay_summary(total["delays"], "onset_delay", "actions", "early_actions", "late_actions")]
    out += [row(last, "fidelity_summary", run_id, app, metric, value, unit) for metric, value, unit in summary]
    return out
