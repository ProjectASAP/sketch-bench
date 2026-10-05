#!/usr/bin/env python3
"""Check that cost measured at a smaller N matches cost measured at the full N.

Re-measures the given points of a serial saturation_cost.jsonl one at a time
at --n-max (the cost phase's approxbench command otherwise) and prints, per
point, the small-N / full-N ratio of:
- insert CPU per item (each run's insert CPU divided by its own N);
- merge CPU per fold (MERGE_SHARDS shards folded into one, same at both N);
- the whole query phase's CPU, as the evaluation tables use it;
- CPU per query (the query phase divided by its query count).

Usage: check_cost_size.py SERIAL_JSONL OUT_JSONL --n-max 1e7 [--seed 1]
           [--binary ./target/release/approxbench] < keys.jsonl
       One JSON key per line on stdin:
       ["cms-fastpath-vector2d", "rows=3 cols=1024", "zipf", 1.0, 10000]
"""

import argparse
import json
import sys

from complete_saturation_costs import dataset_args
from study_saturation import MERGE_SHARDS, cpu_secs, load_saved, point_key, query_secs, run


def size(record):
    return record["workload"]["synthetic"]["description"]["row_num"]


def per_op(record):
    return {
        "insert per item": cpu_secs(record, "insert") / size(record),
        "merge per fold": cpu_secs(record, "merge") / (MERGE_SHARDS - 1),
        "query phase": cpu_secs(record, "query"),
        "per query": query_secs(record),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("serial")
    parser.add_argument("out")
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--n-max", type=float, required=True)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()

    serial = load_saved(args.serial, point_key)
    keys = [json.loads(line) for line in sys.stdin if line.strip()]
    ops = list(per_op(next(iter(serial.values()))))
    print("| sketch | config | θ | K | " + " | ".join(f"{op} small/full N" for op in ops) + " |")
    print("|---|---|---|---|" + "---|" * len(ops))
    worst = 0.0
    with open(args.out, "w") as out:
        for key in keys:
            sketch, config, dist, param, card = key
            row = {"dist": dist, "param": str(param), "cardinality": str(card)}
            record = run(args.binary, [
                "--variant", sketch, "--library", "lib", "--config", config,
                "--operations", "insert,query,merge",
                "--metrics", "throughput,cpu,memory",
                "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
                "--size", str(int(args.n_max)), "--seed", str(args.seed), "--flat",
            ] + dataset_args(row))
            out.write(json.dumps({"key": key, **record}) + "\n")
            out.flush()
            small, full = per_op(record), per_op(serial[point_key(*key)])
            ratios = [small[op] / full[op] if full[op] else float("nan") for op in ops]
            worst = max(worst, *(abs(r - 1) for r in ratios))
            print(f"| {sketch} | {config} | {param:g} | {card} | "
                  + " | ".join(f"{r:.3f}" for r in ratios) + " |", flush=True)
    print(f"\nLargest deviation from the full-N measurement: {worst:.1%}")


if __name__ == "__main__":
    sys.exit(main())
