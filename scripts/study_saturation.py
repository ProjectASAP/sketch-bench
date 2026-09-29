#!/usr/bin/env python3
"""Find the stream length N at which each sketch's accuracy stops changing.

For every (sketch, config, distribution point) the study runs approxbench once
per checkpoint N and seed (a fresh `--size N` stream each time, since accuracy
is only scored after the full stream), averages the error over seeds, and
reports N_sat (see `n_saturation`). At the final N it adds one cost run for
insert/merge/query CPU and memory_bytes, and runs the family's exact polars
baseline at every checkpoint N to find the crossover N* (see `n_star`).

Frequency, top-k and cardinality sketches run on Zipf(theta) over K keys;
quantile sketches run on Pareto(alpha, scale 1000) floored to i64 (the exact
quantile baseline is i64 only), which is unbounded, so K is blank.
`BENCH_WARMUP_SECS` defaults to 0 here: the cost numbers are indicative, and
scripts/export_rqe_optimizer_costs.sh stays the source of optimizer costs.

--phase accuracy runs only the (parallel) accuracy runs; --phase cost reads
saturation_curve.csv from --out and runs only the serial cost runs, so CPU
can be timed later on a quiet machine. Both take the same grid arguments.
--resume keeps the complete curves of an interrupted accuracy run;
--cost-rows 3 times only the rows=3 Vector2D configs.

Writes, under --out:
  saturation_accuracy.jsonl, saturation_cost.jsonl,
  exact_cost.jsonl                                   raw approxbench records
  saturation_curve.csv                               seed-mean error per N
  saturation.csv                                     one row per point (cost
                                                     columns blank after
                                                     --phase accuracy)
  crossover.csv                                      N* per point
"""

import argparse
import csv
import json
import math
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

FREQ_CONFIGS = [f"rows={r} cols={c}" for r in (3, 5) for c in (256, 1024, 4096, 16384)]

# (family, variant, configs, comparator, error metric). Variants follow
# scripts/export_rqe_optimizer_costs.sh; configs step 4x in the size knob
# (asap HLL is compiled at lg_k 12, 14, 16 only, so it has no lg_k 10).
SKETCHES = [
    ("frequency", "cms-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("frequency", "countsketch-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("topk", "cms-heap-topk-fastpath-vector2d", FREQ_CONFIGS, "topk", "precision_at_k"),
    ("cardinality", "hll", ["lg_k=12", "lg_k=14", "lg_k=16"], "cardinality", "relative_error"),
    ("quantile", "kll-percall", ["k=50", "k=200", "k=800"], "rank-error", "mean_rank_err"),
    ("quantile", "dd", ["alpha=0.005", "alpha=0.01", "alpha=0.02", "alpha=0.05"],
     "rank-error", "mean_rank_err"),
]

# The exact polars baseline (variant, config) each family is compared with.
# The config is parsed and ignored; top-k shares the frequency group_by.
EXACT = {
    "frequency": ("cms", "rows=3 cols=256"),
    "topk": ("cms", "rows=3 cols=256"),
    "cardinality": ("hll", "lg_k=12"),
    "quantile": ("kll-cdf", "k=200"),
}
CROSSOVER_FACTORS = (10, 100)

MERGE_SHARDS = 16

SUMMARY_COLUMNS = [
    "family", "sketch", "config", "dist", "param", "cardinality", "n_sat",
    "final_error", "error_metric", "insert_cpu_secs", "merge_cpu_secs",
    "query_cpu_secs", "memory_bytes",
]
CROSSOVER_COLUMNS = [
    "family", "sketch", "config", "dist", "param", "cardinality", "n_sat",
    "sketch_memory_bytes", "sketch_cpu_secs",
] + [f"{m}_n_star_{f}x" for m in ("memory", "cpu") for f in CROSSOVER_FACTORS]
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


def n_star(ns, exact, sketch, factor):
    """Smallest ns[i] with exact[i] >= factor * sketch[i], or None."""
    for n, e, s in zip(ns, exact, sketch):
        if e >= factor * s:
            return n
    return None


def point_key(sketch, config, dist, param, cardinality):
    """How a point is matched across CSVs: param compared as a number."""
    return (sketch, config, dist, float(param), str(cardinality))


def load_curves(path):
    """{point key: sorted [(n, seed-mean error, se)]} from a saturation_curve.csv."""
    curves = {}
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            key = point_key(r["sketch"], r["config"], r["dist"], r["param"], r["cardinality"])
            curves.setdefault(key, []).append(
                (int(r["n"]), float(r["seed_mean_error"]), float(r["seed_se"])))
    return {key: sorted(curve) for key, curve in curves.items()}


def read_curve(path, points, ns, tolerance, tail):
    """(point, n_sat, final error) per point, from an earlier accuracy phase.

    Exits when a point's curve is missing or was measured over other sizes
    than `ns`, i.e. the grid arguments differ from the accuracy run's.
    """
    curves = load_curves(path)
    results = []
    for p in points:
        curve = curves.get(point_key(p[1], p[2], *p[6:]), [])
        if [n for n, _, _ in curve] != ns:
            sys.exit(f"{path} has no curve over these sizes for {p[1]} ({p[2]}) "
                     f"{p[6]}={p[7]} K={p[8]}; pass the accuracy run's grid arguments")
        means = [e for _, e, _ in curve]
        ses = [se for _, _, se in curve]
        results.append((p, n_saturation(ns, means, tolerance, tail, ses), means[-1]))
    return results


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
    parser.add_argument("--phase", choices=["accuracy", "cost", "all"], default="all",
                        help="cost reads saturation_curve.csv from --out")
    parser.add_argument("--points-from",
                        help="CSV with sketch,config,dist,param,cardinality columns "
                             "(e.g. a filtered saturation.csv): run only those points")
    parser.add_argument("--resume", action="store_true",
                        help="accuracy: keep the points whose curve over these sizes is "
                             "already in --out's saturation_curve.csv, run the rest")
    parser.add_argument("--cost-rows", type=int,
                        help="cost: measure only this rows= value of the Vector2D sketches "
                             "(other rows get blank cost columns and no crossover row)")
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
                               "--pareto-scale", "1000", "--cardinality", "1",
                               "--dtype", "i64"]
                    points.append((family, variant, config, comparator, metric,
                                   dataset, "pareto", alpha, ""))
                continue
            for theta in thetas:
                for k in cardinalities:
                    dataset = ["--dataset", "zipf", "--zipf-s", str(theta),
                               "--cardinality", str(k), "--dtype", "i64"]
                    points.append((family, variant, config, comparator, metric,
                                   dataset, "zipf", theta, k))
    if args.points_from:
        with open(args.points_from, newline="") as f:
            wanted = {point_key(r["sketch"], r["config"], r["dist"], r["param"],
                                r["cardinality"]) for r in csv.DictReader(f)}
        points = [p for p in points if point_key(p[1], p[2], *p[6:]) in wanted]
        if len(points) != len(wanted):
            sys.exit(f"--points-from names {len(wanted)} points but only {len(points)} "
                     "are in the grid; widen --families/--thetas/--cardinalities/--alphas")
    print(f"{len(points)} points x {len(ns)} sizes x {len(seeds)} seeds", file=sys.stderr)

    def accuracy(point, n, seed):
        _, variant, config, comparator, metric, dataset, *_ = point
        return run(args.binary, [
            "--variant", variant, "--library", "lib", "--config", config,
            "--operations", "query", "--metrics", "accuracy",
            "--comparator", comparator, "--runs", "1", "--warmup-runs", "0",
            "--size", str(n), "--seed", str(seed),
        ] + dataset)

    if args.phase == "cost":
        results = read_curve(os.path.join(args.out, "saturation_curve.csv"), points, ns,
                             args.tolerance, args.plateau_tail)
    else:
        curve_path = os.path.join(args.out, "saturation_curve.csv")
        # --resume keeps complete curves and writes them back first, so a
        # point cut off mid-write is dropped and rerun, and a second
        # interruption loses nothing that was kept.
        done = {}
        if args.resume and os.path.exists(curve_path):
            done = {key: c for key, c in load_curves(curve_path).items()
                    if [n for n, _, _ in c] == ns}
        raw = open(os.path.join(args.out, "saturation_accuracy.jsonl"),
                   "a" if args.resume else "w")
        curve_file = open(curve_path, "w", newline="")
        curve = csv.writer(curve_file)
        curve.writerow(CURVE_COLUMNS)
        for p in points:
            for n, mean, se in done.get(point_key(p[1], p[2], *p[6:]), []):
                curve.writerow([p[0], p[1], p[2], p[6], p[7], p[8], n, mean, se])
        curve_file.flush()
        # Submit everything up front so the pool stays full; results are consumed
        # in point order, so the curve CSV fills point by point.
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [
                None if point_key(p[1], p[2], *p[6:]) in done else
                [[pool.submit(accuracy, p, n, s) for s in seeds] for n in ns] for p in points
            ]
            print(f"{len(done)} points kept, {sum(f is not None for f in futures)} to run",
                  file=sys.stderr)
            results = []
            for point, per_n in zip(points, futures):
                family, variant, config, _, metric, _, dist, param, k = point
                if per_n is None:
                    kept = done[point_key(variant, config, dist, param, k)]
                    means = [mean for _, mean, _ in kept]
                    n_sat = n_saturation(ns, means, args.tolerance, args.plateau_tail,
                                         [se for _, _, se in kept])
                    results.append((point, n_sat, means[-1]))
                    continue
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
    # export_rqe_optimizer_costs.sh. --phase accuracy leaves these blank.
    rows, records = [], []
    cost = args.phase != "accuracy"
    cost_raw = open(os.path.join(args.out, "saturation_cost.jsonl"), "w") if cost else None
    for point, n_sat, final_error in results:
        family, variant, config, _, metric, dataset, dist, param, k = point
        record = {}
        # rows=5 costs ~5/3 of rows=3 (insert, query and memory are per row).
        skip = args.cost_rows is not None and config.startswith("rows=") \
            and not config.startswith(f"rows={args.cost_rows} ")
        if cost and not skip:
            record = run(args.binary, [
                "--variant", variant, "--library", "lib", "--config", config,
                "--operations", "insert,query,merge",
                "--metrics", "throughput,cpu,memory",
                "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
                "--size", str(ns[-1]), "--seed", str(seeds[0]), "--flat",
            ] + dataset)
            cost_raw.write(json.dumps(record) + "\n")
        records.append(record)
        rows.append([
            family, variant, config, dist, param, k,
            "not_saturated" if n_sat is None else n_sat,
            final_error, metric, cpu_secs(record, "insert"),
            cpu_secs(record, "merge"), cpu_secs(record, "query"),
            record.get("memory_bytes", ""),
        ])
    if cost_raw:
        cost_raw.close()

    with open(os.path.join(args.out, "saturation.csv"), "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(SUMMARY_COLUMNS)
        writer.writerows(rows)
    print(f"Done. {os.path.join(args.out, 'saturation.csv')}", file=sys.stderr)
    if not cost:
        return 0

    # The family's exact baseline at every checkpoint, once per (baseline,
    # dataset). Memory is its memory_bytes after prepare (the buffered stream
    # plus any count table), the same self-reported formula as the sketch's;
    # CPU is insert + prepare (the polars pass) + query.
    exact = {}
    measured = [(r, record) for r, record in zip(results, records) if record]
    with open(os.path.join(args.out, "exact_cost.jsonl"), "w") as exact_raw:
        for (point, _, _), _ in measured:
            variant, config = EXACT[point[0]]
            for n in ns:
                key = (variant, tuple(point[5]), n)
                if key in exact:
                    continue
                base = [
                    "--variant", variant, "--library", "polars", "--config", config,
                    "--runs", "3", "--warmup-runs", "1", "--size", str(n),
                    "--seed", str(seeds[0]), "--flat",
                ] + point[5]
                timed = run(args.binary, base + [
                    "--operations", "insert,query", "--metrics", "throughput,cpu,memory"])
                prepared = run(args.binary, base + [
                    "--operations", "prepare", "--metrics", "latency,cpu,memory"])
                exact_raw.write(json.dumps(timed) + "\n" + json.dumps(prepared) + "\n")
                exact[key] = (prepared["memory_bytes"], cpu_secs(timed, "insert")
                              + cpu_secs(timed, "query") + cpu_secs(prepared, "prepare"))

    with open(os.path.join(args.out, "crossover.csv"), "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(CROSSOVER_COLUMNS)
        for (point, n_sat, _), record in measured:
            family, variant, config, _, _, dataset, dist, param, k = point
            per_n = [exact[(EXACT[family][0], tuple(dataset), n)] for n in ns]
            insert, query = cpu_secs(record, "insert"), cpu_secs(record, "query")
            memory = record["memory_bytes"]
            # Measured at the final N only: insert CPU is a per-item cost, so it
            # scales with N; query CPU and memory are taken as constant.
            sketch_cpu = [insert * n / ns[-1] + query for n in ns]
            stars = [n_star(ns, [m for m, _ in per_n], [memory] * len(ns), x)
                     for x in CROSSOVER_FACTORS]
            stars += [n_star(ns, [c for _, c in per_n], sketch_cpu, x)
                      for x in CROSSOVER_FACTORS]
            writer.writerow([
                family, variant, config, dist, param, k,
                "not_saturated" if n_sat is None else n_sat, memory, insert + query,
            ] + ["not_reached" if n is None else n for n in stars])
    print(f"Done. {os.path.join(args.out, 'crossover.csv')}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
