#!/usr/bin/env python3
"""AutoSketch-Adapted's benchmark time, measured (ProjectASAP/ASAPQuery#777
section 7).

AutoSketch evaluates every configuration its search probes by running it on a
benchmark workload and scoring its accuracy (NSDI '24, section 5.2 and
Algorithm 4 line 5). This runs that benchmark for every distinct probed
(sketch, config, data shape) in the `autosketch_vs_asap` results given:
approxbench's accuracy run at N items (default 1e8, sketch-bench's benchmark
size) on the probe's data shape, including generating the data, the exact
baseline and scoring, and times its wall clock. Each workload is then
charged once per probed (metric, config), as AutoSketch benchmarks a config
on each metric's data, and its result file gets `benchmark_secs_measured`.
Exact accumulators have one configuration and no accuracy to check, so they
cost nothing here.

Usage: scripts/autosketch_benchmark_time.py --binary APPROXBENCH --out TIMES.json RESULT.json...
"""

import argparse
import json
import math
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import SKETCHES, TOPK_K  # noqa: E402

COMPARATOR = {variant: comparator for _, variant, _, comparator, _ in SKETCHES}


def config_string(sketch, params):
    """The approxbench --config of a cost-table row's params."""
    params = dict(params)
    heap = params.pop("heap", None)
    parts = [f"{k}={v}" for k, v in sorted(params.items(), key=lambda kv: kv[0] != "rows")]
    if heap is not None and int(heap) != TOPK_K:
        parts.append(f"topk_k={int(heap)}")
    return " ".join(parts)


def dataset_args(probe):
    if probe["tail_index"] is not None:
        return ["--dataset", "pareto", "--pareto-alpha", str(probe["tail_index"]),
                "--pareto-scale", "1000", "--cardinality", "1", "--dtype", "i64"]
    return ["--dataset", "zipf", "--zipf-s", str(probe["zipf_s"]),
            "--cardinality", str(int(probe["distinct_keys"])), "--dtype", "i64"]


def bench_key(probe):
    """What a benchmark run depends on: the config and the data shape."""
    return json.dumps([probe["sketch"], probe["params"], probe["zipf_s"],
                       probe["distinct_keys"], probe["tail_index"]], sort_keys=True)


def measure(binary, probe, n):
    args = [binary, "sketchbench", "--variant", probe["sketch"], "--library", "lib",
            "--config", config_string(probe["sketch"], probe["params"]),
            "--operations", "query", "--metrics", "accuracy",
            "--comparator", COMPARATOR[probe["sketch"]], "--runs", "1", "--warmup-runs", "0",
            "--size", str(int(n)), "--seed", "1"] + dataset_args(probe)
    started = time.monotonic()
    subprocess.run(args, check=True, capture_output=True, stdin=subprocess.DEVNULL)
    return time.monotonic() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--n", type=float, default=1e8)
    parser.add_argument("--out", required=True, help="per-benchmark wall times (JSON)")
    parser.add_argument("results", nargs="+")
    args = parser.parse_args()
    times = {}
    if os.path.exists(args.out):
        with open(args.out) as f:
            times = json.load(f)["secs"]
    results = {}
    for path in args.results:
        with open(path) as f:
            results[path] = json.load(f)
    probes = {bench_key(p): p for r in results.values()
              for p in r["autosketch"]["probed_configs"] if not p["sketch"].startswith("exact-")}
    for key, probe in sorted(probes.items()):
        if key in times:
            continue
        shape = probe["tail_index"] if probe["tail_index"] is not None else probe["zipf_s"]
        if shape is None or not math.isfinite(shape):
            sys.exit(f"probe {key} has no finite data shape")
        times[key] = measure(args.binary, probe, args.n)
        print(f"{times[key]:8.1f} s  {key}", file=sys.stderr)
        with open(args.out, "w") as f:
            json.dump({"n": args.n, "secs": times}, f, indent=1, sort_keys=True)
    for path, r in results.items():
        a = r["autosketch"]
        a["benchmark_secs_measured"] = sum(
            0.0 if p["sketch"].startswith("exact-") else times[bench_key(p)]
            for p in a["probed_configs"])
        a["benchmark_n"] = args.n
        with open(path, "w") as f:
            json.dump(r, f, indent=1)
        print(f"{path}: {a['benchmark_secs_measured']:.0f} s over "
              f"{len(a['probed_configs'])} probed configs", file=sys.stderr)


if __name__ == "__main__":
    main()
