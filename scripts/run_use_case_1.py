#!/usr/bin/env python3

import argparse
import json
import os
import re
import subprocess
import sys

from approx_rows import row, write_run
from threshold_alert import derive

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

ARMS = {
    "raw-eps0.05": ["--max-error", "0.05"],
    "noraw-eps0.1": ["--no-raw", "--max-error", "0.1"],
    "noraw-eps0.05": ["--no-raw", "--max-error", "0.05"],
    "noraw-eps0.01": ["--no-raw", "--max-error", "0.01"],
    "noraw-ignore-eps0.1": ["--no-raw", "--ignore-guarantees", "--max-error", "0.1"],
    "noraw-ignore-eps0.05": ["--no-raw", "--ignore-guarantees", "--max-error", "0.05"],
    "noraw-ignore-eps0.01": ["--no-raw", "--ignore-guarantees", "--max-error", "0.01"],
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


def answers_by_time(query):
    out = {}
    for a in query["answers"]:
        if a["time"] in out:
            raise ValueError(f"query {query['id']} has two series at {a['time']}; threshold_alert reads one")
        out[a["time"]] = a["value"]
    return out


def query_rows(run_id, use_case, exact_query, approx_query):
    exact = answers_by_time(exact_query)
    approx = answers_by_time(approx_query)
    if set(exact) != set(approx):
        raise ValueError(f"query {exact_query['id']}: exact and approx answered at different times")
    labels = {"use_case": use_case, "query": exact_query["id"], "promql": exact_query["promql"]}
    rows = []
    for t in sorted(exact):
        rows.append(row(t, "query_result", run_id, {**labels, "arm": "exact"}, "value", exact[t], exact_query["unit"]))
        rows.append(row(t, "query_result", run_id, {**labels, "arm": "approx"}, "value", approx[t], exact_query["unit"]))
    return rows


def write_arm(data, use_case, arm, exact, approx):
    run_id = f"uc1-{arm}"
    exact_by_id = {q["id"]: q for q in exact["queries"]}
    rows, thresholds, plans, unsupported = [], {}, {}, {}
    for q in approx["queries"]:
        if "unsupported" in q:
            unsupported[q["id"]] = short_reason(q["unsupported"])
            continue
        rows += query_rows(run_id, use_case, exact_by_id[q["id"]], q)
        thresholds[q["id"]] = q["threshold"]
        plans[q["id"]] = describe_plan(q["plan"])
    if not rows:
        print(f"{run_id}: no query planned; nothing written", file=sys.stderr)
        return
    details = {
        "arm": approx["arm"],
        "plans": plans,
        "unsupported": unsupported,
        "exact_plans": {q["id"]: describe_plan(q["plan"]) for q in exact["queries"] if "plan" in q},
        "planner_rev": approx["planner_rev"],
    }
    write_run(data, run_id, use_case, rows + derive(rows, thresholds), details)
    print(f"{run_id}: {len(plans)} queries, {len(unsupported)} unsupported", file=sys.stderr)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Run use case 1 through ASAPPlanner in process: the exact plan, then each approximate arm"
    )
    parser.add_argument("--spec", default=os.path.join(REPO, "aqpbm-inprocess", "workloads", "uc1_syn_flood.json"))
    parser.add_argument("--binary", default=os.path.join(REPO, "target", "release", "aqpbm-inprocess"))
    parser.add_argument("--data", default=os.path.join(REPO, "target", "grafana-data"))
    parser.add_argument("--raw-out", default=os.path.join(REPO, "target", "inprocess"))
    parser.add_argument("--arm", action="append", choices=sorted(ARMS), help="repeatable; default every arm")
    args = parser.parse_args(argv)
    os.makedirs(args.raw_out, exist_ok=True)
    exact = run_binary(args.binary, args.spec, os.path.join(args.raw_out, "uc1-exact.json"), [])
    for q in exact["queries"]:
        if "unsupported" in q:
            sys.exit(f"the exact plan of {q['id']} is unsupported: {q['unsupported']}")
    for arm in args.arm or list(ARMS):
        approx = run_binary(args.binary, args.spec, os.path.join(args.raw_out, f"uc1-{arm}.json"), ARMS[arm])
        write_arm(args.data, exact["use_case"], arm, exact, approx)


if __name__ == "__main__":
    main()
