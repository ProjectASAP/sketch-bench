#!/usr/bin/env python3

import argparse
import json
from collections import Counter
from pathlib import Path


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def refusal_label(body):
    if not body["refusals"]:
        return body["error"] or ("ok" if body["ran"] else "no record")
    return "; ".join(f"{r['variant']}[{r['reason']}]" for r in body["refusals"])


def families_label(entry):
    seen = []
    for name in entry.get("families", []):
        if name not in seen:
            seen.append(name)
    return ",".join(seen) if seen else "-"


def payload_label(entry):
    return ",".join(sorted(set(entry.get("payloads", [])))) or "-"


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
    undocumented = []
    agreements = []
    for entry in entries:
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
                    str(entry["arm_a_runs"]["datafusion"]),
                    str(entry["arm_b_runs"]["datafusion"]),
                    "ok" if runtimes["interp"]["ran"] else refusal_label(runtimes["interp"]),
                    "ok" if runtimes["datafusion"]["ran"] else refusal_label(runtimes["datafusion"]),
                    "-" if agree is None else str(agree),
                ]
            )
        )
        for runtime in ("interp", "datafusion"):
            body = runtimes[runtime]
            if body["ran"]:
                continue
            reported = body.get("refusal_counts")
            if reported:
                for name, count in reported.items():
                    if count:
                        classes[f"{runtime}.{name}"] += count
            else:
                classes[f"{runtime}.error_before_any_refusal"] += 1
                undocumented.append((entry["query_id"], runtime, body["error"]))
        if runtimes["interp"]["ran"] and runtimes["datafusion"]["ran"]:
            outcomes["both_runtimes_ran"] += 1
            if cross.get("shared_readouts"):
                agreements.append((entry["query_id"], cross))
        elif runtimes["datafusion"]["ran"]:
            outcomes["datafusion_only"] += 1
        elif runtimes["interp"]["ran"]:
            outcomes["interp_only"] += 1
        else:
            outcomes["neither_runtime_ran"] += 1

    print()
    print("outcomes")
    for name, count in sorted(outcomes.items()):
        print(f"  {name:24} {count}")
    print("reason classes")
    for name, count in sorted(classes.items()):
        print(f"  {name:36} {count}")

    if arguments.detail:
        print()
        print("errors carrying no classified refusal")
        for query_id, runtime, error in undocumented:
            print(f"  {query_id} [{runtime}] {error}")
        print()
        print("cross-runtime readout comparisons")
        for query_id, cross in agreements:
            for row in cross["rows"]:
                print(
                    f"  {query_id} node {row['node']} [{row['group']}] {row['query']} "
                    f"interp={row['interp_approximate']} datafusion={row['datafusion_approximate']} "
                    f"identical={row['identical']}"
                )


if __name__ == "__main__":
    main()
