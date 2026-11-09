#!/usr/bin/env python3

import argparse
import json
import os
import re
import subprocess
import sys

import bounds
import ddos_rules
import decision
import rollout
import rate_limit_rules
import rules
import scaling_rules
import threshold_alert
import topk_set
import video_rules
from approx_rows import row, write_run

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SAMPLE_BYTES = 16


def arm_flags(arm):
    flags = []
    for part in arm.split("-"):
        if part == "noraw":
            flags.append("--no-raw")
        elif part == "ignore":
            flags.append("--ignore-guarantees")
        elif part.startswith("eps"):
            flags += ["--max-error", part[3:]]
        elif part != "raw":
            raise ValueError(f"arm {arm}: unknown part {part}")
    return flags


def max_error_of(arm):
    found = [float(part[3:]) for part in arm.split("-") if part.startswith("eps")]
    return found[0] if found else None


def planned_rules(rule_set, queries):
    ids = {q["id"] for q in queries}
    return {name: rule for name, rule in rule_set.items() if set(rule["queries"]) <= ids}


def uc1_derive(rows, queries, policy=bounds.PLAIN, widen=None):
    out = threshold_alert.derive(rows, {q["id"]: q["threshold"] for q in queries}, policy, widen)
    usable = planned_rules(ddos_rules.RULES, queries)
    return out + (rules.derive(rows, usable, policy, widen) if usable else [])


def uc4_derive(rows, queries, policy=bounds.PLAIN, widen=None):
    return rollout.derive(rows, rollout.RULES, policy, widen)


def uc2_derive(rows, queries, policy=bounds.PLAIN, widen=None):
    rule = {"cpu_up": 0.8, "memory_up": 1.5e9, "cpu_down": 0.4, "memory_down": 0.75e9}
    out = decision.derive(rows, "scale_decision", "p95_cpu", "p95_memory", "container", rule, policy, widen)
    return out + rules.derive(rows, scaling_rules.RULES, policy, widen)


def uc5_derive(rows, queries, policy=bounds.PLAIN, widen=None):
    if policy != bounds.PLAIN:
        raise ValueError("the top clients rule has no use for an error bound")
    out = topk_set.derive(rows, "top_clients", "top10_clients", "client_id")
    return out + rules.derive(rows, rate_limit_rules.RULES)


def uc3_derive(rows, queries, policy=bounds.PLAIN, widen=None):
    return rules.derive(rows, video_rules.RULES, policy, widen)


def rule_signals(rule_set):
    return lambda query: [name for name, rule in rule_set.items() if query in rule["queries"]]


def uc4_stage2(derived, errors):
    ranked = {r["labels"]["query"] for r in errors}
    relative = [r for r in derived if r["kind"] == "low_level" and r["metric"] == "relative_error"
                and r["labels"]["query"] not in ranked]
    return rollout_errors(derived, errors + relative, rollout.RULES)


USE_CASES = {
    "1": {
        "spec": "uc1_syn_flood.json",
        "arms": ["raw-eps0.05", "noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01",
                 "noraw-ignore-eps0.1", "noraw-ignore-eps0.05", "noraw-ignore-eps0.01"],
        "signal": lambda query: query,
        "signals": lambda query: [query] + rule_signals(ddos_rules.RULES)(query),
        "rules": ddos_rules.RULES,
        "derive": uc1_derive,
        "dashboard": "threshold-alert",
        "needs_every_query": False,
        "split": {"app_state": "entity"},
    },
    "2": {
        "spec": "uc2_autoscaling.json",
        "arms": ["raw-eps0.01", "noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005"],
        "planeval_flags": ["--executor-max-bytes", str(2 << 30)],
        "signal": lambda query: "scale_decision",
        "signals": lambda query: (["scale_decision"] if query in ("p95_cpu", "p95_memory") else [])
        + rule_signals(scaling_rules.RULES)(query),
        "rules": scaling_rules.RULES,
        "derive": uc2_derive,
        "dashboard": "decision",
        "needs_every_query": True,
        "split": {"query_result": "container", "low_level": "container", "app_state": ("container", "entity")},
    },
    "3": {
        "spec": "uc3_video.json",
        "arms": ["noraw-ignore-eps0.1", "noraw-ignore-eps0.05", "noraw-ignore-eps0.01"],
        "planeval_flags": ["--executor-max-bytes", str(1 << 30)],
        "signal": lambda query: "video",
        "signals": rule_signals(video_rules.RULES),
        "rules": video_rules.RULES,
        "derive": uc3_derive,
        "dashboard": "rules",
        "needs_every_query": True,
        "split": {"query_result": "city", "low_level": "city", "app_state": "entity"},
    },
    "4": {
        "spec": "uc4_canary.json",
        "arms": ["raw-eps0.01", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005", "noraw-eps0.001"],
        "signal": lambda query: "rollout",
        "signals": lambda query: [name for name, rule in rollout.RULES.items() if query in rule["queries"]],
        "derive": uc4_derive,
        "dashboard": "rollout",
        "needs_every_query": True,
        "split": {},
        "stage2": uc4_stage2,
    },
    "5": {
        "spec": "uc5_top_clients.json",
        "arms": ["noraw-ignore-eps0.05", "noraw-ignore-eps0.01", "noraw-ignore-eps0.005", "noraw-ignore-eps0.001"],
        "signal": lambda query: "top_clients",
        "signals": lambda query: (["top_clients"] if query == "top10_clients" else [])
        + rule_signals(rate_limit_rules.RULES)(query),
        "rules": rate_limit_rules.RULES,
        "derive": uc5_derive,
        "dashboard": "topk",
        "bounds": False,
        "needs_every_query": True,
        "split": {"app_state": ("client_id", "entity")},
    },
}


POLICY_TEXT = {
    bounds.BOUND: "An answer whose error-bound interval leaves the check open takes no action",
    bounds.RERUN: "An answer whose error-bound interval leaves the check open reruns the check's queries with "
                  "the exact plan",
}


def run_binary(binary, spec, out, flags):
    cmd = [binary, "--spec", spec, "--out", out] + flags
    print("$ " + " ".join(cmd), file=sys.stderr)
    subprocess.run(cmd, check=True)
    with open(out) as f:
        return json.load(f)


def describe_plan(plan):
    sketches = [
        f"{s.get('algorithm', '?')} {json.dumps(s.get('params'), separators=(',', ':'))}"
        for s in plan["sketches"]
    ]
    refresh = [v["refresh"] for v in plan["materialized_views"]]
    where = "query time" if not refresh else (
        f"{refresh.count('at_ingestion')} views at ingestion" if "at_ingestion" in refresh
        else f"{len(refresh)} views kept across evaluations")
    return f"{'; '.join(sketches) or 'exact'} ({where})"


def sketch_bounds(query):
    plan = query["plan"]
    rows = []
    for sketch, analytics in zip(plan["sketches"], plan.get("sketch_analytics", [])):
        for readout in analytics["readouts"] or [{}]:
            error = readout.get("error") or {}
            rows.append({
                "query": query["id"],
                "sketch": f"{sketch.get('algorithm', '?')} {json.dumps(sketch.get('params'), separators=(',', ':'))}",
                "statistic": readout.get("statistic"),
                "bound metric": error.get("metric"),
                "epsilon": error.get("epsilon"),
                "delta": error.get("delta"),
                "merged error": readout.get("merged_error"),
                "sketchlib bytes per state": analytics["memory_bytes"],
                "planner peak bytes for the plan": plan["predicted_peak_memory_bytes"],
                "update ops": analytics["update_ops"],
            })
    return rows


def short_reason(reason):
    m = re.search(r'choice \d+: ([^"]+?)(?: AccuracyTarget|")', reason)
    return m.group(1) if m else reason[:160]


def query_rows(run_id, use_case, signal, exact_query, approx_query):
    labels = {"use_case": use_case, "signal": signal, "query": exact_query["id"]}
    rows = []
    for arm, query in (("exact", exact_query), ("approx", approx_query)):
        for a in query["answers"]:
            series = {k: v for k, v in a["series"].items() if k != "__name__"}
            rows.append(row(a["time"], "query_result", run_id, {**labels, **series, "arm": arm}, "value",
                            a["value"], exact_query["unit"]))
    return sorted(rows, key=lambda r: r["time"])


def unnamed(series):
    return {k: v for k, v in series.items() if k != "__name__"}


def exact_plan_errors(run_id, use_case, signal, approx_query, error, metric, unit):
    labels = {"use_case": use_case, "signal": signal, "query": approx_query["id"]}
    return [row(a["time"], "low_level", run_id, {**labels, **unnamed(a["series"])}, metric,
                a["exact_plan"][error], unit)
            for a in approx_query["answers"] if error in a.get("exact_plan", {})]


def rank_errors(run_id, use_case, signal, approx_query):
    return exact_plan_errors(run_id, use_case, signal, approx_query, "rank_error", "rank_error", "rank")


def relative_errors(run_id, use_case, signal, approx_query):
    return exact_plan_errors(run_id, use_case, signal, approx_query, "relative_error", "relative_error", "ratio")


def query_costs(run_id, use_case, signal, exact_query, approx_query):
    labels = {"use_case": use_case, "signal": signal, "query": exact_query["id"]}
    rows = []
    for arm, query in (("exact", exact_query), ("approx", approx_query)):
        for c in query.get("costs", []):
            rows.append(row(c["time"], "low_level", run_id, {**labels, "arm": arm}, "query_latency",
                            c["query_seconds"], "s"))
            rows.append(row(c["time"], "low_level", run_id, {**labels, "arm": arm}, "query_memory",
                            c["state_bytes"] + c["samples_read_at_query"] * SAMPLE_BYTES, "bytes"))
    return rows


def mean(values):
    return sum(values) / len(values) if values else None


def costs_of(query):
    costs = query.get("costs", [])
    refresh = sum(c["refresh_seconds"] for c in costs)
    return {
        "mean query ms": mean([c["query_seconds"] * 1e3 for c in costs]),
        "mean compile ms": mean([c["compile_seconds"] * 1e3 for c in costs]),
        "planning ms": query.get("plan", {}).get("planning_ms"),
        "insert samples per second": sum(c["samples_ingested"] for c in costs) / refresh if refresh else None,
        "peak state bytes": max((c["state_bytes"] for c in costs), default=None),
        "mean samples read at query": mean([c["samples_read_at_query"] for c in costs]),
        "peak raw sample bytes read at query": max((c["samples_read_at_query"] * SAMPLE_BYTES for c in costs),
                                                   default=None),
    }


def stage1(arm, exact_query, approx_query, derived):
    query = exact_query["id"]
    ranked = any("rank_error" in a.get("exact_plan", {}) for a in approx_query["answers"])
    metric = "rank_error" if ranked else "relative_error"
    errors = [r["value"] for r in derived
              if r["kind"] == "low_level" and r["metric"] == metric and r["labels"]["query"] == query]
    target = arm["max_error"]
    out = {"query": query, "error metric": metric, "target": target,
           "failure probability": arm["failure_probability"],
           "max error": max(errors, default=None), "mean error": mean(errors), "evaluations": len(errors),
           "evaluations over target": None if target is None else sum(e > target for e in errors)}
    out.update(costs_of(approx_query))
    out.update({f"exact {k}": v for k, v in costs_of(exact_query).items()})
    return out


def rollout_errors(derived, errors, rule_set):
    by_query = {}
    for r in errors:
        by_query.setdefault(r["labels"]["query"], {})[r["time"]] = r["value"]
    out = []
    for name, rule in rule_set.items():
        times = sorted({r["time"] for r in derived
                        if r["kind"] == "app_state" and r["labels"].get("signal") == name})
        for r in derived:
            if r["kind"] != "fidelity_event" or r["metric"] != "rollout" or r["labels"].get("signal") != name:
                continue
            first = times.index(r["time"])
            span = times[first:first + rule["checks"]]
            out.append({"rule": name, "rollout": int(r["labels"]["rollout"]), "outcome": r["value"],
                        "exact": r["labels"]["exact"], "approx": r["labels"]["approx"],
                        "max error": {q: max(e[t] for t in span if t in e) for q, e in sorted(by_query.items())
                                      if q in rule["queries"]}})
    return sorted(out, key=lambda x: (x["rule"], x["rollout"]))


def delay_text(s, metric, name):
    return f"; {name} {s[metric]:.2f}" if metric in s else ""


def signed_delay_text(s, metric, name, early, late):
    if metric not in s:
        return ""
    return f"; {name} {s[metric]:.2f} ({s[early]} early, {s[late]} late)"


def counted(n, noun, plural=None):
    return f"{n} {noun}" if n == 1 else f"{n} {plural or noun + 's'}"


def headline(app, s):
    if app == threshold_alert.APP:
        total = s["true_positive"] + s["false_positive"] + s["false_negative"] + s["true_negative"]
        return ((s["true_positive"] + s["true_negative"]) / total,
                f"{s['false_positive']} false positive and {s['false_negative']} false negative evaluations; "
                f"{s['missed_episodes']} of {counted(s['episodes'], 'episode')} missed; {counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay")
                + signed_delay_text(s, "mean_absolute_end_delay", "mean absolute end delay", "early_ends",
                                    "late_ends"))
    if app == decision.APP:
        return (s["action_agreement"],
                f"{s['true_positive']} matched, {s['false_positive']} false and {s['false_negative']} missed "
                f"scaling actions ({s['up_true_positive']} up, {s['down_true_positive']} down matched)"
                + signed_delay_text(s, "mean_absolute_onset_delay", "mean absolute onset delay", "early_actions",
                                    "late_actions"))
    if app == rollout.APP:
        return (s["agreement"],
                f"{s['true_positive'] + s['true_negative']} of {counted(s['rollouts'], 'rollout')} decided the same; "
                f"{counted(s['false_positive'], 'false rollback')}, "
                f"{counted(s['false_negative'], 'missed rollback')}; {counted(s['gaps'], 'gap')} in failed checks"
                + signed_delay_text(s, "mean_absolute_rollback_delay", "mean absolute rollback delay",
                                    "early_rollbacks", "late_rollbacks"))
    if app == rules.APP:
        return (s["agreement"],
                f"{s['true_positive']} true positive, {s['false_positive']} false positive and "
                f"{s['false_negative']} false negative entity-evaluations over "
                f"{counted(s['entities'], 'entity', 'entities')}; "
                f"{s['missed_entries']} of {counted(s['entries'], 'episode')} missed; {counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay"))
    if app == topk_set.APP:
        size = s["mean_list_size"]
        return (s["mean_overlap"] / size,
                f"mean overlap {s['mean_overlap']:.2f} of "
                f"{counted(int(size) if float(size).is_integer() else round(size, 2), 'client')}; "
                f"identical lists {s['identical_sets']} of {s['evaluations']}; "
                f"{s['true_positive']} true positive, {s['false_positive']} false positive and "
                f"{s['false_negative']} false negative client-evaluations; precision {s['precision']:.3f}; "
                f"{s['missed_entries']} of {counted(s['entries'], 'entry', 'entries')} missed; "
                f"{counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay"))
    raise ValueError(f"no headline for application {app}")


def headlines(derived):
    by_signal = {}
    for r in derived:
        if r["kind"] == "fidelity_summary" and r["metric"] != "disagreements":
            key = (r["labels"]["app"], r["labels"]["signal"])
            by_signal.setdefault(key, {})[r["metric"]] = r["value"]
    out = {}
    for (app, signal), summary in by_signal.items():
        agreement, text = headline(app, summary)
        if "unsure_checks" in summary:
            text += f"; {counted(summary['unsure_checks'], 'unsure check')}"
        out[signal] = {"application": app, "agreement": agreement, "fidelity": text}
    return out


def max_relative_errors(derived):
    out = {}
    for r in derived:
        if r["kind"] == "low_level" and r["metric"] == "relative_error":
            query = r["labels"]["query"]
            out[query] = max(out.get(query, 0.0), r["value"])
    return out


def run_name(prefix, arm, trial, policy):
    name = f"{prefix}-{arm}" if policy == bounds.PLAIN else f"{prefix}-{arm}-{policy}"
    return name if trial == 1 else f"{name}-trial{trial}"


def write_arm(data, case, prefix, arm, exact, approx, trial=1, policy=bounds.PLAIN, widen=None):
    run_id = run_name(prefix, arm, trial, policy)
    use_case = exact["use_case"]
    exact_by_id = {q["id"]: q for q in exact["queries"]}
    rows, errors, relative, costs, planned, plans, unsupported = [], [], [], [], [], {}, {}
    for q in approx["queries"]:
        if bounds.is_bound_query(q["id"]):
            continue
        if "unsupported" in q:
            unsupported[q["id"]] = short_reason(q["unsupported"])
            continue
        rows += query_rows(run_id, use_case, case["signal"](q["id"]), exact_by_id[q["id"]], q)
        errors += rank_errors(run_id, use_case, case["signal"](q["id"]), q)
        relative += relative_errors(run_id, use_case, case["signal"](q["id"]), q)
        costs += query_costs(run_id, use_case, case["signal"](q["id"]), exact_by_id[q["id"]], q)
        planned.append(q)
        plans[q["id"]] = describe_plan(q["plan"])
    if not rows or unsupported and case["needs_every_query"]:
        print(f"{run_id}: queries {sorted(unsupported)} have no plan; nothing written", file=sys.stderr)
        return
    details = {
        "arm": approx["arm"],
        "trial": trial,
        "plans": plans,
        "unsupported": unsupported,
        "exact_plans": {q["id"]: describe_plan(q["plan"]) for q in exact["queries"] if "plan" in q},
        "promql": {q["id"]: q["promql"] for q in exact["queries"]},
        "planner_rev": approx["planner_rev"],
        "dashboard": case["dashboard"],
        "signals": {q["id"]: case.get("signals", case["signal"])(q["id"]) for q in exact["queries"]},
    }
    if "rules" in case:
        details["rules"] = {name: rule["description"] for name, rule in case["rules"].items()}
        details["signal_dashboards"] = {name: "rules" for name in case["rules"]}
    if policy != bounds.PLAIN:
        details["policy"] = POLICY_TEXT[policy]
        details["bounded_queries"] = widen.bounded_queries()
    derived = case["derive"](rows, planned, policy, widen) + relative + errors
    details["max_relative_error"] = max_relative_errors(derived)
    details["sketch_bounds"] = [r for q in planned for r in sketch_bounds(q)]
    details["fidelity"] = headlines(derived)
    details["stage1"] = [stage1(approx["arm"], exact_by_id[q["id"]], q, derived) for q in planned]
    if "stage2" in case:
        details["stage2"] = case["stage2"](derived, errors)
    write_run(data, run_id, use_case, rows + derived + costs, details, case["split"])
    print(f"{run_id}: {len(plans)} queries, {len(unsupported)} unsupported", file=sys.stderr)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Run a use case through ASAPPlanner in process: the exact plan, then each approximate arm"
    )
    parser.add_argument("use_case", choices=sorted(USE_CASES))
    parser.add_argument("--binary", default=os.path.join(REPO, "target", "release", "planeval"))
    parser.add_argument("--data", default=os.path.join(REPO, "target", "grafana-data"))
    parser.add_argument("--raw-out", default=os.path.join(REPO, "target", "planeval"))
    parser.add_argument("--arm", action="append", help="repeatable; default every arm of the use case")
    parser.add_argument("--trials", type=int, default=1,
                        help="runs of each approximate arm, each with fresh sketch seeds")
    parser.add_argument("--policy", action="append", choices=bounds.POLICIES,
                        help="repeatable; how a rule treats an answer whose error bound leaves its check open: "
                             "plain ignores the bound, bound takes no action, rerun asks the exact plan; "
                             "default every policy the use case supports")
    args = parser.parse_args(argv)
    case = USE_CASES[args.use_case]
    prefix = f"uc{args.use_case}"
    spec = os.path.join(REPO, "aqpbm-planeval", "workloads", case["spec"])
    for arm in args.arm or []:
        if arm not in case["arms"]:
            parser.error(f"arm {arm} is not one of {case['arms']}")
    os.makedirs(args.raw_out, exist_ok=True)
    exact_out = os.path.join(args.raw_out, f"{prefix}-exact.json")
    extra = case.get("planeval_flags", [])
    exact = run_binary(args.binary, spec, exact_out, extra)
    for q in exact["queries"]:
        if "unsupported" in q:
            sys.exit(f"the exact plan of {q['id']} is unsupported: {q['unsupported']}")
    policies = args.policy or (bounds.POLICIES if case.get("bounds", True) else (bounds.PLAIN,))
    if not case.get("bounds", True) and set(policies) != {bounds.PLAIN}:
        parser.error(f"use case {args.use_case} supports only the plain policy")
    for arm in args.arm or case["arms"]:
        max_error = max_error_of(arm)
        arm_spec = spec
        if max_error is not None and set(policies) != {bounds.PLAIN}:
            with open(spec) as f:
                widened = bounds.with_bound_queries(json.load(f), max_error)
            arm_spec = os.path.join(args.raw_out, f"{prefix}-{arm}-spec.json")
            with open(arm_spec, "w") as f:
                json.dump(widened, f)
        for trial in range(1, args.trials + 1):
            name = arm if trial == 1 else f"{arm}-trial{trial}"
            approx = run_binary(args.binary, arm_spec, os.path.join(args.raw_out, f"{prefix}-{name}.json"),
                                arm_flags(arm) + extra + ["--exact", exact_out])
            widen = bounds.Widen(approx, max_error) if max_error is not None else None
            for policy in policies:
                if policy != bounds.PLAIN and not (widen and widen.bounded_queries()):
                    print(f"{run_name(prefix, arm, trial, policy)}: no plan has an error bound; not written",
                          file=sys.stderr)
                    continue
                write_arm(args.data, case, prefix, arm, exact, approx, trial, policy, widen)

if __name__ == "__main__":
    main()
