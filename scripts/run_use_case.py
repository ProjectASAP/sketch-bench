#!/usr/bin/env python3

import argparse
import json
import os
import re
import subprocess
import sys

import comparison_alert
import decision
import threshold_alert
import topk_set
from approx_rows import row, write_run

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


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


def uc1_derive(rows, queries):
    return threshold_alert.derive(rows, {q["id"]: q["threshold"] for q in queries})


def uc4_derive(rows, queries):
    return comparison_alert.derive(rows, "canary_slower", "p99_canary", "p99_stable", 1.1)


def uc2_derive(rows, queries):
    rule = {"cpu_up": 0.8, "memory_up": 1.5e9, "cpu_down": 0.3, "memory_down": 0.5e9}
    return decision.derive(rows, "scale_decision", "p95_cpu", "p95_memory", "container", rule)


def uc5_derive(rows, queries):
    return topk_set.derive(rows, "top_clients", "top10_clients", "client_id")


USE_CASES = {
    "1": {
        "spec": "uc1_syn_flood.json",
        "arms": ["raw-eps0.05", "noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01",
                 "noraw-ignore-eps0.1", "noraw-ignore-eps0.05", "noraw-ignore-eps0.01"],
        "signal": lambda query: query,
        "derive": uc1_derive,
        "dashboard": "threshold-alert",
        "needs_every_query": False,
    },
    "2": {
        "spec": "uc2_autoscaling.json",
        "arms": ["raw-eps0.01", "noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005"],
        "signal": lambda query: "scale_decision",
        "derive": uc2_derive,
        "dashboard": "decision",
        "needs_every_query": True,
    },
    "4": {
        "spec": "uc4_canary.json",
        "arms": ["raw-eps0.01", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005", "noraw-eps0.001"],
        "signal": lambda query: "canary_slower",
        "derive": uc4_derive,
        "dashboard": "threshold-alert",
        "needs_every_query": True,
    },
    "5": {
        "spec": "uc5_top_clients.json",
        "arms": ["noraw-ignore-eps0.05", "noraw-ignore-eps0.01", "noraw-ignore-eps0.005", "noraw-ignore-eps0.001"],
        "signal": lambda query: "top_clients",
        "derive": uc5_derive,
        "dashboard": "topk",
        "needs_every_query": True,
    },
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


def headline(app, s):
    if app in (threshold_alert.APP, comparison_alert.APP):
        total = s["true_positive"] + s["false_positive"] + s["false_negative"] + s["true_negative"]
        return ((s["true_positive"] + s["true_negative"]) / total,
                f"{s['false_positive']} false positive and {s['false_negative']} false negative evaluations; "
                f"{s['missed_episodes']} of {s['episodes']} episodes missed; {s['breaks']} breaks")
    if app == decision.APP:
        return (s["agreement"],
                f"{s['exact_none_approx_up']} extra and {s['exact_up_approx_none']} missing scale-ups; "
                f"{s['exact_none_approx_down']} extra and {s['exact_down_approx_none']} missing scale-downs; "
                f"{s['up_missed_episodes'] + s['down_missed_episodes']} of "
                f"{s['up_episodes'] + s['down_episodes']} scaling episodes missed")
    if app == topk_set.APP:
        return (s["identical_sets"] / s["evaluations"],
                f"precision {s['precision']:.3f}; {s['intruder_slots']} approx-only slots; "
                f"{s['missed_entries']} of {s['entries']} entries missed")
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
        out[signal] = {"application": app, "agreement": agreement, "fidelity": text}
    return out


def max_relative_errors(derived):
    out = {}
    for r in derived:
        if r["kind"] == "low_level" and r["metric"] == "relative_error":
            query = r["labels"]["query"]
            out[query] = max(out.get(query, 0.0), r["value"])
    return out


def write_arm(data, case, prefix, arm, exact, approx):
    run_id = f"{prefix}-{arm}"
    use_case = exact["use_case"]
    exact_by_id = {q["id"]: q for q in exact["queries"]}
    rows, planned, plans, unsupported = [], [], {}, {}
    for q in approx["queries"]:
        if "unsupported" in q:
            unsupported[q["id"]] = short_reason(q["unsupported"])
            continue
        rows += query_rows(run_id, use_case, case["signal"](q["id"]), exact_by_id[q["id"]], q)
        planned.append(q)
        plans[q["id"]] = describe_plan(q["plan"])
    if not rows or unsupported and case["needs_every_query"]:
        print(f"{run_id}: queries {sorted(unsupported)} have no plan; nothing written", file=sys.stderr)
        return
    details = {
        "arm": approx["arm"],
        "plans": plans,
        "unsupported": unsupported,
        "exact_plans": {q["id"]: describe_plan(q["plan"]) for q in exact["queries"] if "plan" in q},
        "promql": {q["id"]: q["promql"] for q in exact["queries"]},
        "planner_rev": approx["planner_rev"],
        "dashboard": case["dashboard"],
        "signals": {q["id"]: case["signal"](q["id"]) for q in exact["queries"]},
    }
    derived = case["derive"](rows, planned)
    details["max_relative_error"] = max_relative_errors(derived)
    details["fidelity"] = headlines(derived)
    write_run(data, run_id, use_case, rows + derived, details)
    print(f"{run_id}: {len(plans)} queries, {len(unsupported)} unsupported", file=sys.stderr)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Run a use case through ASAPPlanner in process: the exact plan, then each approximate arm"
    )
    parser.add_argument("use_case", choices=sorted(USE_CASES))
    parser.add_argument("--binary", default=os.path.join(REPO, "target", "release", "aqpbm-inprocess"))
    parser.add_argument("--data", default=os.path.join(REPO, "target", "grafana-data"))
    parser.add_argument("--raw-out", default=os.path.join(REPO, "target", "inprocess"))
    parser.add_argument("--arm", action="append", help="repeatable; default every arm of the use case")
    args = parser.parse_args(argv)
    case = USE_CASES[args.use_case]
    prefix = f"uc{args.use_case}"
    spec = os.path.join(REPO, "aqpbm-inprocess", "workloads", case["spec"])
    for arm in args.arm or []:
        if arm not in case["arms"]:
            parser.error(f"arm {arm} is not one of {case['arms']}")
    os.makedirs(args.raw_out, exist_ok=True)
    exact = run_binary(args.binary, spec, os.path.join(args.raw_out, f"{prefix}-exact.json"), [])
    for q in exact["queries"]:
        if "unsupported" in q:
            sys.exit(f"the exact plan of {q['id']} is unsupported: {q['unsupported']}")
    for arm in args.arm or case["arms"]:
        approx = run_binary(args.binary, spec, os.path.join(args.raw_out, f"{prefix}-{arm}.json"), arm_flags(arm))
        write_arm(args.data, case, prefix, arm, exact, approx)

if __name__ == "__main__":
    main()
