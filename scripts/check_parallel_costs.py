#!/usr/bin/env python3
"""Check that cost measured --jobs at a time matches serial measurement.

Re-measures the given points of a serial saturation_cost.jsonl, all at once
on --jobs workers, with the cost phase's approxbench command, and prints the
parallel/serial ratio of insert, merge and query CPU seconds per point.

Usage: check_parallel_costs.py SERIAL_JSONL OUT_JSONL [--jobs 12]
           [--n-max 1e8] [--seed 1] [--binary ./target/release/approxbench]
       Points are read from stdin, one JSON key per line:
       ["cms-fastpath-vector2d", "rows=3 cols=1024", "zipf", 1.0, 10000]
"""

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor

from complete_saturation_costs import dataset_args
from study_saturation import MERGE_SHARDS, cpu_secs, load_saved, point_key, run

OPS = ("insert", "merge", "query")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("serial")
    parser.add_argument("out")
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--jobs", type=int, default=12)
    parser.add_argument("--n-max", type=float, default=1e8)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()

    serial = load_saved(args.serial, point_key)
    keys = [json.loads(line) for line in sys.stdin if line.strip()]

    def measure(key):
        sketch, config, dist, param, card = key
        row = {"dist": dist, "param": str(param), "cardinality": str(card)}
        return key, run(args.binary, [
            "--variant", sketch, "--library", "lib", "--config", config,
            "--operations", "insert,query,merge",
            "--metrics", "throughput,cpu,memory",
            "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
            "--size", str(int(args.n_max)), "--seed", str(args.seed), "--flat",
        ] + dataset_args(row))

    print("| sketch | config | θ | K | " + " | ".join(f"{op} parallel/serial" for op in OPS) + " |")
    print("|---|---|---|---|" + "---|" * len(OPS))
    worst = 0.0
    with ThreadPoolExecutor(max_workers=args.jobs) as pool, open(args.out, "w") as out:
        for key, record in pool.map(measure, keys):
            out.write(json.dumps({"key": key, **record}) + "\n")
            base = serial[point_key(*key)]
            ratios = [cpu_secs(record, op) / cpu_secs(base, op) for op in OPS]
            worst = max(worst, *(abs(r - 1) for r in ratios))
            print(f"| {key[0]} | {key[1]} | {key[3]:g} | {key[4]} | "
                  + " | ".join(f"{r:.3f}" for r in ratios) + " |")
    print(f"\nLargest deviation from serial: {worst:.1%}")


if __name__ == "__main__":
    sys.exit(main())
