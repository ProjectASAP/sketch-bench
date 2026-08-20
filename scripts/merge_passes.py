#!/usr/bin/env python3
"""
merge_passes.py — merge multi-pass JSONL records into one row per config.

The bench runner emits one record per metric pass:
  - insert pass: throughput_items_per_sec + cpu/wall/rss/heap/memory_bytes
  - query pass:  throughput_items_per_sec + accuracy + cpu/wall/rss/heap/memory_bytes
                 (cpu/wall/rss/heap are absent or null for impls with no accuracy comparator)
The pass is named by the record's `bench.operation`, not guessed from which
fields are present.

This script groups by (sketch, impl, language, sketch_config, workload, mode)
and merges into a single flat record with prefixed fields.

Usage:
  python3 scripts/merge_passes.py output/profiles/all.jsonl > output/profiles/merged.jsonl
  python3 scripts/merge_passes.py output/profiles/all.jsonl -o output/profiles/merged.jsonl
"""

import json
import sys
import argparse
from collections import defaultdict


def phase(record):
    b = record.get("bench") or {}
    op = b.get("operation")
    if op == "insert":
        return "insert"
    return "query"  # memory-only second pass — still the query/accuracy slot


def config_key(record):
    sc = record.get("sketch_config") or {}
    return (
        record["sketch"],
        record["impl"],
        record["language"],
        json.dumps(sc, sort_keys=True),
        json.dumps(record["workload"], sort_keys=True),
        record["mode"],
    )


def merge_group(records):
    by_phase = {}
    for r in records:
        p = phase(r)
        # last writer wins if somehow two insert or two query records appear
        by_phase[p] = r

    base = by_phase.get("insert") or by_phase.get("query")
    insert_bench = (by_phase.get("insert") or {}).get("bench") or {}
    query_bench = (by_phase.get("query") or {}).get("bench") or {}

    out = {
        "schema_version": base["schema_version"],
        "sketch": base["sketch"],
        "impl": base["impl"],
        "language": base["language"],
        "sketch_config": base.get("sketch_config"),
        "workload": base["workload"],
        "mode": base["mode"],
        "runs": base["runs"],
        "source": base["source"],
        # memory_bytes is the sketch's structural footprint — same across passes
        "memory_bytes": insert_bench.get("memory_bytes") or query_bench.get("memory_bytes"),
        # insert-phase metrics
        "insert_throughput_items_per_sec": insert_bench.get("throughput_items_per_sec"),
        "insert_cpu_time_ms": insert_bench.get("cpu_time_ms"),
        "insert_wall_time_ms": insert_bench.get("wall_time_ms"),
        "insert_rss_peak_kb": insert_bench.get("rss_peak_kb"),
        "insert_heap_allocated_kb": insert_bench.get("heap_allocated_kb"),
        # query/accuracy-phase metrics (null for impls with no accuracy comparator)
        "query_throughput_items_per_sec": query_bench.get("throughput_items_per_sec"),
        "query_cpu_time_ms": query_bench.get("cpu_time_ms"),
        "query_wall_time_ms": query_bench.get("wall_time_ms"),
        "query_rss_peak_kb": query_bench.get("rss_peak_kb"),
        "query_heap_allocated_kb": query_bench.get("heap_allocated_kb"),
        "accuracy": query_bench.get("accuracy"),
    }
    # drop explicit nulls to keep output lean
    return {k: v for k, v in out.items() if v is not None}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("input", help="input JSONL file (- for stdin)")
    ap.add_argument("-o", "--output", help="output file (default: stdout)")
    args = ap.parse_args()

    src = sys.stdin if args.input == "-" else open(args.input)
    dst = open(args.output, "w") if args.output else sys.stdout

    groups = defaultdict(list)
    order = []  # preserve insertion order of first-seen keys
    for line in src:
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        k = config_key(r)
        if k not in groups:
            order.append(k)
        groups[k].append(r)

    for k in order:
        dst.write(json.dumps(merge_group(groups[k])) + "\n")

    if args.output:
        dst.close()
    if args.input != "-":
        src.close()

    print(f"merged {sum(len(v) for v in groups.values())} records → {len(order)} rows", file=sys.stderr)


if __name__ == "__main__":
    main()
