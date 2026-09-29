#!/usr/bin/env python3
"""Find the stream length N at which each sketch's accuracy stops changing.

For every (sketch, config, distribution point) the study runs approxbench once
per checkpoint N and seed (a fresh `--size N` stream each time, since accuracy
is only scored after the full stream), averages the error over seeds, and
reports N_sat (see `n_saturation`). At the final N it adds one cost run for
insert/merge/query CPU and memory_bytes.

Frequency, top-k and cardinality sketches run on Zipf(theta) over K keys;
quantile sketches run on Pareto(alpha), which is unbounded, so K is blank.
`BENCH_WARMUP_SECS` defaults to 0 here: the cost numbers are indicative, and
scripts/export_rqe_optimizer_costs.sh stays the source of optimizer costs.

Writes, under --out:
  saturation_accuracy.jsonl, saturation_cost.jsonl  raw approxbench records
  saturation_curve.csv                               seed-mean error per N
  saturation.csv                                     one row per point
"""

import argparse
import csv
import json
import math
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

FREQ_CONFIGS = [f"rows={r} cols={c}" for r in (3, 5) for c in (1024, 2048)]

# (family, variant, configs, comparator, error metric). Variants and configs
# follow scripts/export_rqe_optimizer_costs.sh.
SKETCHES = [
    ("frequency", "cms-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("frequency", "countsketch-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("topk", "cms-heap-topk-fastpath-vector2d", FREQ_CONFIGS, "topk", "precision_at_k"),
    ("cardinality", "hll", ["lg_k=12", "lg_k=14"], "cardinality", "relative_error"),
    ("quantile", "kll-percall", ["k=200", "k=500"], "rank-error", "mean_rank_err"),
    ("quantile", "dd", ["alpha=0.01", "alpha=0.02"], "rank-error", "mean_rank_err"),
]

MERGE_SHARDS = 16

SUMMARY_COLUMNS = [
    "family", "sketch", "config", "dist", "param", "cardinality", "n_sat",
    "final_error", "error_metric", "insert_cpu_secs", "merge_cpu_secs",
    "query_cpu_secs", "memory_bytes",
]
CURVE_COLUMNS = [
    "family", "sketch", "config", "dist", "param", "cardinality", "n",
    "seed_mean_error", "seed_se",
]


def checkpoints(n_min, n_max, per_decade):
    """Log-spaced sizes from n_min to n_max inclusive, per_decade per decade."""
    lo, hi = math.log10(n_min), math.log10(n_max)
    steps = round((hi - lo) * per_decade)
    ns = [round(10 ** (lo + i / per_decade)) for i in range(steps + 1)]
    return sorted(set(ns))


def n_saturation(ns, errors, tolerance, tail, ses=None, abs_tol=1e-9):
    """Smallest checkpoint from which the error stays on its plateau.

    The plateau is the mean of the last `tail` seed-mean errors. A checkpoint
    is in band when |error - plateau| <= max(tolerance * |plateau|, 2 * se,
    abs_tol), where se is that checkpoint's standard error across seeds
    (`ses`, 0 when omitted), so seed noise alone never breaks the plateau;
    abs_tol only matters when the plateau is (near) zero. N_sat is ns[i] for
    the smallest i such that every checkpoint i..end is in band. Comparing
    magnitudes makes direction irrelevant (precision_at_k rises, errors fall).

    Returns None ("not_saturated") when there are fewer than `tail`
    checkpoints, or when no checkpoint before the tail qualifies (including a
    tail that is not itself in band): the tail agreeing with its own mean says
    nothing about saturation.
    """
    if len(errors) < tail:
        return None
    plateau = sum(errors[-tail:]) / tail
    ses = ses or [0.0] * len(errors)
    first = len(errors)
    for i in range(len(errors) - 1, -1, -1):
        band = max(tolerance * abs(plateau), 2 * ses[i], abs_tol)
        if abs(errors[i] - plateau) > band:
            break
        first = i
    if first > len(errors) - tail - 1:
        return None
    return ns[first]


def mean_and_se(values):
    """Mean and standard error of the mean (0 for a single value)."""
    mean = sum(values) / len(values)
    if len(values) < 2:
        return mean, 0.0
    var = sum((v - mean) ** 2 for v in values) / (len(values) - 1)
    return mean, math.sqrt(var / len(values))


def run(binary, args):
    """Run approxbench sketchbench and return the last JSONL record it prints."""
    out = subprocess.run(
        [binary, "sketchbench"] + args,
        check=True, capture_output=True, text=True, stdin=subprocess.DEVNULL,
    ).stdout
    return json.loads(out.strip().splitlines()[-1])


def error(record, metric):
    acc = record["bench"]["accuracy"]
    if metric == "are_top100" and metric not in acc:
        # Fewer than 100 distinct keys were seen, so the top 100 is all of them.
        return acc["are_all"]
    return acc[metric]


def cpu_secs(record, op):
    cpu = record.get(f"{op}_cpu_time_ms")
    if not cpu:
        return ""
    return (cpu["user_ms"]["mean"] + cpu["sys_ms"]["mean"]) / 1000.0


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--out", default="out")
    parser.add_argument("--families", default="frequency,topk,cardinality,quantile")
    parser.add_argument("--one-config", action="store_true",
                        help="only the first config of each sketch")
    parser.add_argument("--thetas", default="0,0.5,0.8,1.0,1.2,1.5,2.0")
    parser.add_argument("--cardinalities", default="1000,100000,10000000")
    parser.add_argument("--alphas", default="1.1,1.5,2,3")
    parser.add_argument("--n-min", type=float, default=1e3)
    parser.add_argument("--n-max", type=float, default=1e8)
    parser.add_argument("--per-decade", type=int, default=4)
    parser.add_argument("--seeds", type=int, default=5)
    parser.add_argument("--tolerance", type=float, default=0.10)
    parser.add_argument("--plateau-tail", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=1,
                        help="parallel accuracy runs (cost runs are always serial)")
    args = parser.parse_args()

    families = args.families.split(",")
    thetas = [float(t) for t in args.thetas.split(",")]
    cardinalities = [int(k) for k in args.cardinalities.split(",")]
    alphas = [float(a) for a in args.alphas.split(",")]
    ns = checkpoints(args.n_min, args.n_max, args.per_decade)
    seeds = list(range(1, args.seeds + 1))
    os.environ.setdefault("BENCH_WARMUP_SECS", "0")
    os.makedirs(args.out, exist_ok=True)

    # A point is (family, variant, config, comparator, metric, dataset args,
    # dist, param, cardinality).
    points = []
    for family, variant, configs, comparator, metric in SKETCHES:
        if family not in families:
            continue
        for config in configs[:1] if args.one_config else configs:
            if family == "quantile":
                for alpha in alphas:
                    # --cardinality is required by the CLI but ignored for pareto.
                    dataset = ["--dataset", "pareto", "--pareto-alpha", str(alpha),
                               "--cardinality", "1", "--dtype", "f64"]
                    points.append((family, variant, config, comparator, metric,
                                   dataset, "pareto", alpha, ""))
                continue
            for theta in thetas:
                for k in cardinalities:
                    dataset = ["--dataset", "zipf", "--zipf-s", str(theta),
                               "--cardinality", str(k), "--dtype", "i64"]
                    points.append((family, variant, config, comparator, metric,
                                   dataset, "zipf", theta, k))
    print(f"{len(points)} points x {len(ns)} sizes x {len(seeds)} seeds", file=sys.stderr)

    def accuracy(point, n, seed):
        _, variant, config, comparator, metric, dataset, *_ = point
        return run(args.binary, [
            "--variant", variant, "--library", "lib", "--config", config,
            "--operations", "query", "--metrics", "accuracy",
            "--comparator", comparator, "--runs", "1", "--warmup-runs", "0",
            "--size", str(n), "--seed", str(seed),
        ] + dataset)

    raw = open(os.path.join(args.out, "saturation_accuracy.jsonl"), "w")
    curve_file = open(os.path.join(args.out, "saturation_curve.csv"), "w", newline="")
    curve = csv.writer(curve_file)
    curve.writerow(CURVE_COLUMNS)
    # Submit everything up front so the pool stays full; results are consumed
    # in point order, so the curve CSV fills point by point.
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [
            [[pool.submit(accuracy, p, n, s) for s in seeds] for n in ns] for p in points
        ]
        results = []
        for point, per_n in zip(points, futures):
            family, variant, config, _, metric, _, dist, param, k = point
            means, ses = [], []
            for n, per_seed in zip(ns, per_n):
                records = [f.result() for f in per_seed]
                for r in records:
                    raw.write(json.dumps(r) + "\n")
                mean, se = mean_and_se([error(r, metric) for r in records])
                means.append(mean)
                ses.append(se)
                curve.writerow([family, variant, config, dist, param, k, n, mean, se])
            curve_file.flush()
            n_sat = n_saturation(ns, means, args.tolerance, args.plateau_tail, ses)
            results.append((point, n_sat, means[-1]))
            print(f"  {variant} ({config}) {dist}={param} K={k}: n_sat={n_sat}",
                  file=sys.stderr)
    raw.close()
    curve_file.close()

    # Cost at the final N, one serial run per point with the merge setup of
    # export_rqe_optimizer_costs.sh.
    rows = []
    with open(os.path.join(args.out, "saturation_cost.jsonl"), "w") as cost_raw:
        for point, n_sat, final_error in results:
            family, variant, config, _, metric, dataset, dist, param, k = point
            record = run(args.binary, [
                "--variant", variant, "--library", "lib", "--config", config,
                "--operations", "insert,query,merge",
                "--metrics", "throughput,cpu,memory",
                "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
                "--size", str(ns[-1]), "--seed", str(seeds[0]), "--flat",
            ] + dataset)
            cost_raw.write(json.dumps(record) + "\n")
            rows.append([
                family, variant, config, dist, param, k,
                "not_saturated" if n_sat is None else n_sat,
                final_error, metric, cpu_secs(record, "insert"),
                cpu_secs(record, "merge"), cpu_secs(record, "query"),
                record.get("memory_bytes", ""),
            ])

    with open(os.path.join(args.out, "saturation.csv"), "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(SUMMARY_COLUMNS)
        writer.writerows(rows)
    print(f"Done. {os.path.join(args.out, 'saturation.csv')}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
