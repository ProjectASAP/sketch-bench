#!/usr/bin/env python3
"""Finish an interrupted `study_saturation.py --phase cost` run.

The cost phase measures one point at a time and rewrites
saturation_cost.jsonl from scratch, so an interrupted run cannot resume.
This runs the points still missing from saturation_cost.jsonl (the same
approxbench command as the cost phase, --jobs at a time), appends them, and
fills the cost columns of saturation.csv from every record. It does not run
the exact-baseline crossover.

--n-max is the stream length of the new measurements. Readers of
saturation.csv divide insert CPU by the point's last curve N, so each
record's insert CPU is written scaled to --normalize-n (default: --n-max),
keeping insert CPU per item equal to the record's own CPU / its own N.

Usage: complete_saturation_costs.py GRID_DIR [--jobs 12] [--cost-rows 3]
           [--n-max 1e8] [--normalize-n 1e8] [--seed 1]
           [--binary ./target/release/approxbench]
"""

import argparse
import csv
import json
import os
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from study_saturation import (  # noqa: E402
    MERGE_SHARDS, SUMMARY_COLUMNS, cpu_secs, load_saved, point_key, run)


def dataset_args(row):
    """The cost phase's dataset arguments for one saturation.csv row."""
    if row["dist"] == "pareto":
        return ["--dataset", "pareto", "--pareto-alpha", row["param"],
                "--pareto-scale", "1000", "--cardinality", "1", "--dtype", "i64"]
    return ["--dataset", "zipf", "--zipf-s", row["param"],
            "--cardinality", row["cardinality"], "--dtype", "i64"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("grid_dir")
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--jobs", type=int, default=12)
    parser.add_argument("--cost-rows", type=int, default=3)
    parser.add_argument("--n-max", type=float, default=1e8)
    parser.add_argument("--normalize-n", type=float,
                        help="N the insert CPU is scaled to (default: --n-max)")
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    normalize_n = args.normalize_n or args.n_max

    csv_path = os.path.join(args.grid_dir, "saturation.csv")
    cost_path = os.path.join(args.grid_dir, "saturation_cost.jsonl")
    rows = list(csv.DictReader(open(csv_path)))
    saved = load_saved(cost_path, point_key) if os.path.exists(cost_path) else {}

    def key(row):
        return point_key(row["sketch"], row["config"], row["dist"], row["param"],
                         row["cardinality"])

    def measured(row):
        config = row["config"]
        return not (config.startswith("rows=")
                    and not config.startswith(f"rows={args.cost_rows} "))

    missing = [r for r in rows if measured(r) and key(r) not in saved]
    print(f"{len(saved)} saved, {len(missing)} to run", file=sys.stderr)

    def measure(row):
        return row, run(args.binary, [
            "--variant", row["sketch"], "--library", "lib", "--config", row["config"],
            "--operations", "insert,query,merge",
            "--metrics", "throughput,cpu,memory",
            "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
            "--size", str(int(args.n_max)), "--seed", str(args.seed), "--flat",
        ] + dataset_args(row))

    with ThreadPoolExecutor(max_workers=args.jobs) as pool, open(cost_path, "a") as out:
        for row, record in pool.map(measure, missing):
            k = key(row)
            out.write(json.dumps({"key": [row["sketch"], row["config"], row["dist"],
                                          float(row["param"]), row["cardinality"]],
                                  **record}) + "\n")
            out.flush()
            saved[k] = record
            print(f"  {row['sketch']} ({row['config']}) {row['dist']}={row['param']} "
                  f"K={row['cardinality']}", file=sys.stderr)

    for row in rows:
        record = saved.get(key(row), {}) if measured(row) else {}
        insert = cpu_secs(record, "insert")
        size = record.get("workload", {}).get("synthetic", {}).get("description", {}).get("row_num")
        row["insert_cpu_secs"] = insert * normalize_n / size if insert != "" and size else insert
        row["merge_cpu_secs"] = cpu_secs(record, "merge")
        row["query_cpu_secs"] = cpu_secs(record, "query")
        row["memory_bytes"] = record.get("memory_bytes", "")
    with open(csv_path, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=SUMMARY_COLUMNS)
        writer.writeheader()
        writer.writerows(rows)
    print(f"Done. {csv_path}", file=sys.stderr)


if __name__ == "__main__":
    sys.exit(main())
