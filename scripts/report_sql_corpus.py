#!/usr/bin/env python3

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

REASON_CLASSES = ("promql_only", "time_axis", "no_constructor", "deferred", "unclassified")
RUNTIMES = ("interp", "datafusion")


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def refusal_label(body):
    if body["status"] == "ran":
        return "ok"
    if body["refusals"]:
        return "; ".join(f"{r['variant']}[{r['reason']}]" for r in body["refusals"])
    return body.get("error") or body["status"]


def families_label(entry):
    seen = []
    for name in entry.get("families", []):
        if name not in seen:
            seen.append(name)
    return ",".join(seen) if seen else "-"


def payload_label(entry):
    return ",".join(sorted(set(entry.get("payloads", [])))) or "-"


def flag(value):
    return "-" if value is None else str(value)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("jsonl", nargs="+")
    parser.add_argument("--detail", action="store_true")
    arguments = parser.parse_args()

    entries = []
    for path in arguments.jsonl:
        entries.extend(load(path))

    header = [
        "query",
        "lowers",
        "plans",
        "payloads",
        "families",
        "armA_df",
        "armB_df",
        "interp",
        "datafusion",
        "agree",
    ]
    print("\t".join(header))
    classes = Counter()
    outcomes = Counter()
    arms = Counter()
    lowered = 0
    planned = 0
    unreadable = []
    agreements = []
    for entry in entries:
        if entry.get("status") == "output_not_understood":
            print(
                "\t".join(
                    [entry["query_id"], "-", "-", "-", "-", "-", "-", "unreadable", "unreadable", "-"]
                )
            )
            outcomes["output_not_understood"] += 1
            unreadable.append((entry["query_id"], entry["complaint"]))
            continue
        if entry["lowers"]:
            lowered += 1
        if entry["plans"]:
            planned += 1
        runtimes = entry.get("runtimes")
        if runtimes is None:
            print(
                "\t".join(
                    [
                        entry["query_id"],
                        str(entry["lowers"]),
                        "False",
                        "-",
                        "-",
                        "-",
                        "-",
                        "not reached",
                        "not reached",
                        "-",
                    ]
                )
            )
            outcomes["plan_refused"] += 1
            classes["plan_refused"] += 1
            continue
        cross = entry.get("cross_runtime") or {}
        agree = cross.get("agree")
        print(
            "\t".join(
                [
                    entry["query_id"],
                    str(entry["lowers"]),
                    str(entry["plans"]),
                    payload_label(entry),
                    families_label(entry),
                    flag(entry["arm_a_runs"]["datafusion"]),
                    flag(entry["arm_b_runs"]["datafusion"]),
                    refusal_label(runtimes["interp"]),
                    refusal_label(runtimes["datafusion"]),
                    "-" if agree is None else str(agree),
                ]
            )
        )
        for runtime in RUNTIMES:
            body = runtimes[runtime]
            if entry["arm_a_runs"][runtime]:
                arms[f"{runtime}.arm_a"] += 1
            if entry["arm_b_runs"][runtime]:
                arms[f"{runtime}.arm_b"] += 1
            if body["status"] == "ran":
                continue
            for name, count in body["refusal_counts"].items():
                if count:
                    classes[f"{runtime}.{name}"] += count
            if not any(body["refusal_counts"].values()):
                classes[f"{runtime}.{body['status']}_without_a_refusal"] += 1
        ran = {runtime: runtimes[runtime]["status"] == "ran" for runtime in RUNTIMES}
        if ran["interp"] and ran["datafusion"]:
            outcomes["both_runtimes_ran"] += 1
            if cross.get("shared_readouts"):
                agreements.append((entry["query_id"], cross))
        elif ran["datafusion"]:
            outcomes["datafusion_only"] += 1
        elif ran["interp"]:
            outcomes["interp_only"] += 1
        else:
            outcomes["neither_runtime_ran"] += 1

    refused_queries = Counter()
    for entry in entries:
        for runtime in RUNTIMES:
            body = (entry.get("runtimes") or {}).get(runtime)
            if body and any(body["refusal_counts"].values()):
                refused_queries[runtime] += 1

    print()
    print(f"queries                  {len(entries)}")
    print(f"  lower                  {lowered}")
    print(f"  plan                   {planned}")
    print(f"  lower but do not plan  {lowered - planned}")
    print("outcomes")
    for name, count in sorted(outcomes.items()):
        print(f"  {name:24} {count}")
    print("reason classes")
    for name, count in sorted(classes.items()):
        print(f"  {name:40} {count}")
    print("queries carrying at least one refusal")
    for runtime in RUNTIMES:
        print(f"  {runtime:24} {refused_queries[runtime]}")
    print("queries whose arm executed")
    for name, count in sorted(arms.items()):
        print(f"  {name:24} {count}")
    print(f"cross-runtime accuracy comparisons  {len(agreements)}")

    if arguments.detail:
        print()
        print("output the sweep could not read")
        for query_id, complaint in unreadable:
            print(f"  {query_id} {complaint}")
        print()
        print("cross-runtime readout comparisons")
        for query_id, cross in agreements:
            for row in cross["rows"]:
                print(
                    f"  {query_id} node {row['node']} [{row['group']}] {row['query']} "
                    f"interp={row['interp_approximate']} datafusion={row['datafusion_approximate']} "
                    f"identical={row['identical']}"
                )

    return 1 if unreadable else 0


if __name__ == "__main__":
    sys.exit(main())
