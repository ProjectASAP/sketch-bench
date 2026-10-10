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
`BENCH_WARMUP_SECS` defaults to 0 here: the per-point cost numbers are
indicative. The optimizer's costs come from --phase optimizer-cost (#174).

--phase accuracy runs only the (parallel) accuracy runs; --phase cost reads
saturation_curve.csv from --out and runs only the serial cost runs, so CPU
can be timed later on a quiet machine. Both take the same grid arguments.
Top-k curves run at each k in --topk-ks (default 10, 32, 100); a config at
k != 32 carries ` topk_k=k` in its config string, so the CSVs key k there.
Older accuracy files (k = 32 only) read unchanged, but --phase cost and
crossover over them need --topk-ks 32, so the grid matches the curves.
--phase optimizer-cost measures each (sketch, config) of the families the
optimizer plans (OPTIMIZER_FAMILIES) once, serially, at one
shape (COST_*, the synthetic evaluation's dataset) with a cost and an
accuracy pass, plus the exact accumulators, and reduces them to
rqe_atomic_costs.json, the table rqe-optimizer loads; every row names its
accuracy_metric. It runs no curves and ignores the grid arguments.
The grid also holds every config at the optimizer-cost shape unless
--no-cost-shape: θ = 1.1 and K = 1e4 join --thetas and --cardinalities (a full
row and column, so the grid stays a full cross) and a = 2 joins --alphas.
--phase hydra measures Hydra's per-group error on the eval's datasets and a
generic hierarchy (HYDRA_SWEEPS) into hydra_saturation.csv; see `hydra`.
--resume keeps the complete curves of an interrupted accuracy run;
--cost-rows 3 times only the rows=3 Vector2D configs.

Writes, under --out:
  saturation_accuracy.jsonl, saturation_cost.jsonl,
  exact_cost.jsonl                                   raw approxbench records
                                                     (cost ones keyed by point)
  saturation_curve.csv                               seed-mean error per N
  saturation.csv                                     one row per point (cost
                                                     columns blank after
                                                     --phase accuracy)
  crossover.csv                                      N* per point
  saturation_merge_curve.csv                         with --merge-shards-list:
      seed-mean error per N of the sketch merged from m contiguous shards
      (m=1 is the plain single-sketch query)
  rqe_atomic_costs.json (+ _raw, _grid .jsonl)       --phase optimizer-cost only
  hydra_saturation.csv (+ hydra_accuracy.jsonl)      --phase hydra only: one row
      per run, HYDRA_COLUMNS
"""

import argparse
import csv
import io
import itertools
import json
import math
import os
import re
import subprocess
import sys
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed

FREQ_CONFIGS = [f"rows={r} cols={c}" for r in (3, 5) for c in (256, 1024, 4096, 16384)]

# (family, variant, configs, comparator, error metric). The error metric is
# also the cost table's accuracy_metric; configs step 4x in the size knob
# (asap HLL is compiled at lg_k 12, 14, 16 only, so it has no lg_k 10).
SKETCHES = [
    ("frequency", "cms-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("frequency", "countsketch-fastpath-vector2d", FREQ_CONFIGS, "frequency", "are_top100"),
    ("topk", "cms-heap-topk-fastpath-vector2d", FREQ_CONFIGS, "topk", "precision_at_k"),
    ("cardinality", "hll", ["lg_k=12", "lg_k=14", "lg_k=16"], "cardinality", "relative_error"),
    ("quantile", "kll-percall", ["k=50", "k=200", "k=800"], "rank-error", "mean_rank_err"),
    ("quantile", "dd", ["alpha=0.005", "alpha=0.01", "alpha=0.02", "alpha=0.05"],
     # Mean over probes, like KLL: both answer the same quantile query.
     "relative-value-error", "mean_relative_value_error"),
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

# --phase optimizer-cost: the one data shape every sketch's cost is measured
# at, the synthetic evaluation's dataset (ProjectASAP/ASAPQuery#777):
# Zipf(1.1) over 1e4 keys, quantiles on Pareto(a = 2, scale 1000), 1e6
# items. Cost is taken as independent of the shape (#174).
COST_THETA = 1.1
COST_KEYS = 10_000
COST_PARETO_ALPHA = 2.0
COST_N = 1_000_000
COST_RUNS, COST_WARMUP = 5, 3
COST_SEED = 42
COST_TABLE = "rqe_atomic_costs.json"
# Families some rqe-optimizer capability plans; frequency sketches serve none.
OPTIMIZER_FAMILIES = ("topk", "cardinality", "quantile")
# Heap capacities the cost table measures each top-k config at. A deployment
# merging m windows keeps a heap of m · k (k = 32, the graded and answered
# k); rqe-optimizer interpolates costs between these. Accuracy curves are
# measured at the default heap, k.
TOPK_HEAPS = (32, 128, 512, 2048)
# The k a top-k answer is graded at: curves run at each, a config at k
# carrying ` topk_k=k` (its heap defaults to k). TOPK_K is left implicit,
# so its points keep their old keys. Kept equal to rqe-optimizer's TOPK_K
# and sketch-bench's (the cost table's top-k rows are graded at it).
TOPK_K = 32
TOPK_KS = (10, 32, 100)

# Exact accumulators the optimizer plans, on grouped records (variant,
# comparator, datagen spec). Their error is 0 by construction; the accuracy
# pass checks it. They report the worst group's relative_error.
EXACT_COST_ROWS = [
    ("exact-sum", "sum-or-count", "configs/datagen/hydra_columns.yaml"),
    ("exact-min", "min", "configs/datagen/hydra_columns.yaml"),
    ("exact-max", "max", "configs/datagen/hydra_columns.yaml"),
    ("exact-increase", "rate-or-increase", "configs/datagen/counter_columns.yaml"),
    ("exact-delta-set", "key-set", "configs/datagen/hydra_columns.yaml"),
]
EXACT_METRIC = "relative_error"

# --phase hydra (docs/rqe_optimizer_hydra.md; ASAPQuery#777 §6.3): Hydra's
# per-group error depends on the group's share of the fanned-out mass, so the
# eval reads accuracy measured on datasets with its own schemas
# (configs/datagen/hydra_http*.yaml, hydra_flows.yaml, matching
# export_autosketch_eval_table.py's SCHEMAS). Each run asks one grouping (by
# label name, every non-empty subset of the schema) at grid R x W, N records,
# merged from m interleaved shards (1 = the plain query), one seed, and writes
# its per-group errors (--per-group-out), from which the coverage columns are
# read. The other statistics run on the generic hydra_hier*.yaml sweep
# (schema width 2, 3, 4).
HYDRA_R = 3
# Inner cells: the canonical ones (docs/rqe_optimizer_hydra.md §1), except
# UnivMon's, whose 5 x 2048 x 8-layer cell (~650 KB) makes a 3 x 16384 grid
# 32 GB; this one is ~100 KB.
HYDRA_UNIVMON_CELL = "cell_heap_size=64 cell_sketch_row=3 cell_sketch_col=256 cell_layer_size=8"
HYDRA_CELLS = {
    "hydra-cms": "cell_rows=3 cell_cols=512",
    "hydra-cs": "cell_rows=3 cell_cols=512",
    "hydra-hll": "",
    "hydra-kll": "cell_k=200",
    "hydra-kll-cdf": "cell_k=200",
    **{f"hydra-univmon-{s}": HYDRA_UNIVMON_CELL
       for s in ("cardinality", "l1-norm", "l2-norm", "entropy", "sum")},
}
# The statistic each variant answers, a column of hydra_saturation.csv.
HYDRA_STATISTICS = {
    "hydra-cms": "frequency", "hydra-cs": "frequency", "hydra-hll": "cardinality",
    "hydra-kll": "quantile", "hydra-kll-cdf": "cdf",
    "hydra-univmon-cardinality": "cardinality", "hydra-univmon-l1-norm": "l1_norm",
    "hydra-univmon-l2-norm": "l2_norm", "hydra-univmon-entropy": "entropy",
    "hydra-univmon-sum": "sum",
}
# Bytes per grid cell, measured at N = 1e6 (data and truth excluded), and per
# record of generated data plus its exact per-group truth: --hydra-mem-gb
# admits a run only while the running ones' estimates fit. A merge holds all
# m shard grids at once.
HYDRA_CELL_BYTES = {"hydra-cms": 6_500, "hydra-cs": 6_500, "hydra-hll": 17_000,
                    "hydra-kll": 10_000, "hydra-kll-cdf": 10_000}
HYDRA_UNIVMON_CELL_BYTES = 100_000
HYDRA_RECORD_BYTES = 250
# (sweep, datasets as (yaml stem, value dtype, variants), W, N, merge shards,
# seeds).
HYDRA_SWEEPS = {
    "eval": ([("hydra_http", "i64", ["hydra-hll", "hydra-univmon-cardinality"]),
              ("hydra_flows", "i64", ["hydra-hll", "hydra-univmon-cardinality"]),
              ("hydra_http_latency", "f64", ["hydra-kll"])],
             (1024, 4096, 16384), (100_000, 1_000_000, 10_000_000), (1, 4, 16), 3),
    "generic": ([(stem, "i64", list(HYDRA_CELLS))
                 for stem in ("hydra_hier_d2", "hydra_hier_d3", "hydra_hier")],
                (1024, 4096), (1_000_000,), (1, 4), 2),
}
HYDRA_DATAGEN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "configs",
                             "datagen")
HYDRA_TABLE = "hydra_saturation.csv"
# Coverage thresholds τ: err_max_cov_τ is the worst error over the groups of
# share >= τ plus the largest group (the eval's covers_share rule).
HYDRA_COVERS = (0.01, 0.05)
HYDRA_COLUMNS = [
    "variant", "statistic", "config", "R", "W", "dataset", "group_columns", "schema_width",
    "records", "fanned_mass", "merge_shards", "merge_split", "seed", "err_mean", "err_p50",
    "err_p90", "err_max", "groups_scored",
] + [f"err_max_cov_{tau}" for tau in HYDRA_COVERS]
# What makes a row the same measurement, for --resume.
HYDRA_KEY = ["variant", "config", "dataset", "group_columns", "records", "merge_shards",
             "seed"]
# The optimizer-cost table's Hydra rows: the eval's variants on its datasets
# at each eval W. Their accuracy column is the default grouping's err_max; the
# optimizer reads Hydra accuracy from hydra_saturation.csv.
HYDRA_COST_METRIC = "err_max"
# Fewer passes than COST_RUNS: one pass inserts COST_N records into up to
# 2^4 - 1 subsets each, and a UnivMon grid's merge pass rebuilds 16 of them.
HYDRA_COST_RUNS = (3, 1)

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
MERGE_CURVE_COLUMNS = CURVE_COLUMNS[:-2] + ["shards", "seed_mean_error", "seed_se"]


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
    """Smallest ns[i] from which exact >= factor * sketch holds at every later
    checkpoint, or None. "From which it stays" rather than "first time":
    fixed per-run overheads (polars start-up) can make the exact side look
    expensive at the smallest N and cheap again further up."""
    star = None
    for n, e, s in zip(reversed(ns), reversed(exact), reversed(sketch)):
        if e < factor * s:
            break
        star = n
    return star


def point_key(sketch, config, dist, param, cardinality):
    """How a point is matched across CSVs: param and cardinality compared as
    numbers, so a CSV round-tripped through pandas ("1000.0") still matches."""
    card = str(int(float(cardinality))) if str(cardinality) != "" else ""
    return (sketch, config, dist, float(param), card)


def load_curves(path, shards=False):
    """{point key: sorted [(n, seed-mean error, se)]} from a saturation_curve.csv,
    or {point key + (m,): ...} from a saturation_merge_curve.csv with `shards`."""
    curves = {}
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            key = point_key(r["sketch"], r["config"], r["dist"], r["param"], r["cardinality"])
            if shards:
                key += (int(r["shards"]),)
            curves.setdefault(key, []).append(
                (int(r["n"]), float(r["seed_mean_error"]), float(r["seed_se"])))
    return {key: sorted(curve) for key, curve in curves.items()}


def load_saved(path, key):
    """{key(*record["key"]): record} from a cost JSONL of keyed records."""
    with open(path) as f:
        records = [json.loads(line) for line in f]
    return {key(*r.pop("key")): r for r in records}


def saved_record(saved, key, path):
    """The saved record for `key`, or exit when the cost phase never ran it."""
    if key not in saved:
        sys.exit(f"{path} has no record for {key}; rerun --phase cost with this "
                 "grid and --cost-rows")
    return saved[key]


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
                     f"{p[6]}={p[7]} K={p[8]}; pass the accuracy run's grid arguments "
                     "(an accuracy run without the optimizer-cost shape needs "
                     "--no-cost-shape; one from before top-k k was a dimension, "
                     "--topk-ks 32)")
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


def query_secs(record):
    """CPU seconds of one query: the query phase repeats the query over its
    probes (all keys seen, 101 quantiles, or a fixed repeat count), and the
    exact baseline uses a different probe count, so both sides are compared
    per query."""
    cpu, wall = record.get("query_cpu_time_ms"), record.get("query_wall_time_ms")
    rate = record.get("query_throughput_items_per_sec")
    if not (cpu and wall and rate):
        return 0.0
    ops = rate["mean"] * wall["mean"] / 1000.0
    return cpu_secs(record, "query") / ops if ops else 0.0


def cpu_secs(record, op):
    cpu = record.get(f"{op}_cpu_time_ms")
    if not cpu:
        return ""
    return (cpu["user_ms"]["mean"] + cpu["sys_ms"]["mean"]) / 1000.0


def cost_dataset(family):
    """The approxbench data arguments of --phase optimizer-cost's shape."""
    if family == "quantile":
        return ["--dataset", "pareto", "--pareto-alpha", str(COST_PARETO_ALPHA),
                "--pareto-scale", "1000", "--cardinality", "1", "--dtype", "i64",
                "--size", str(COST_N)]
    return ["--dataset", "zipf", "--zipf-s", str(COST_THETA), "--cardinality",
            str(COST_KEYS), "--dtype", "i64", "--size", str(COST_N)]


def optimizer_cost(args, families):
    """The optimizer's cost table: one serial cost pass and one accuracy pass
    per (sketch, config) at the COST_* shape, plus the exact accumulators,
    reduced by `approxbench atomic-costs` to --out/COST_TABLE. Each sketch
    row's accuracy is then the mean over seeds 1..--seeds at COST_N, the
    same measurement as the curve's point there when --seeds matches the
    accuracy run's, so check_cost_table compares like with like. Exact rows
    keep their 0."""
    os.makedirs(args.out, exist_ok=True)
    raw = os.path.join(args.out, "rqe_atomic_costs_raw.jsonl")
    grid = os.path.join(args.out, "rqe_atomic_costs_grid.jsonl")
    table = os.path.join(args.out, COST_TABLE)
    for path in (raw, grid, table):
        if os.path.exists(path):
            sys.exit(f"{path} exists; pick a new --out")

    rows, metrics = [], {}
    for family, variant, configs, comparator, metric in SKETCHES:
        if family not in families or family not in OPTIMIZER_FAMILIES:
            continue
        metrics[variant] = metric
        for config in configs[:1] if args.one_config else configs:
            heaps = [f" heap={h}" for h in TOPK_HEAPS] if family == "topk" else [""]
            for heap in heaps:
                rows.append((variant, "lib", ["--config", config + heap], comparator,
                             cost_dataset(family)))
    for variant, comparator, spec in EXACT_COST_ROWS:
        metrics[variant] = EXACT_METRIC
        rows.append((variant, "exact", [], comparator, ["--spec", spec, "--dtype", "i64"]))
    # Hydra: each eval variant on its datasets at the full schema, at each
    # eval W, COST_N records (the spec as written, seed 1). Insert is per
    # record at that schema width, merge per grid, query per probe, memory
    # per grid; measured_at names the dataset.
    hydra_datasets = {}
    if "hydra" in families:
        datasets, ws = HYDRA_SWEEPS["eval"][0], HYDRA_SWEEPS["eval"][1]
        for stem, dtype, variants in datasets:
            with open(os.path.join(args.datagen, f"{stem}.yaml")) as f:
                text = f.read()
            spec = os.path.join(args.out, f"{stem}_n{COST_N}.yaml")
            with open(spec, "w") as f:
                f.write(hydra_spec(text, COST_N, 1))
            for variant in variants:
                metrics[variant] = HYDRA_COST_METRIC
                for w in ws[:1] if args.one_config else ws:
                    config = hydra_config(variant, w)
                    rows.append((variant, "lib", ["--config", config], None,
                                 ["--spec", spec, "--dtype", dtype]))
                    # The reducer reads the schema width off the scores; the
                    # dataset is the one thing the record cannot name.
                    key = (variant, w, len(hydra_labels(text)))
                    if key in hydra_datasets:
                        sys.exit(f"Hydra cost rows {key} on {hydra_datasets[key]} and {stem} "
                                 "cannot be told apart")
                    hydra_datasets[key] = stem

    def sketchbench(variant, library, config, data, passes):
        subprocess.run(
            [args.binary, "sketchbench", "--variant", variant, "--library", library]
            + config + passes + data + ["--seed", str(COST_SEED), "--report", raw],
            check=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)

    for variant, library, config, comparator, data in rows:
        print(f"  {variant} {' '.join(config[1:])}", file=sys.stderr)
        # Cost first: flatten keeps the first pass's query timings.
        runs, warmup = HYDRA_COST_RUNS if variant.startswith("hydra-") else \
            (COST_RUNS, COST_WARMUP)
        sketchbench(variant, library, config, data, [
            "--operations", "insert,query,merge", "--metrics", "throughput,cpu,memory",
            "--merge-shards", str(MERGE_SHARDS), "--runs", str(runs),
            "--warmup-runs", str(warmup)])
        sketchbench(variant, library, config, data, [
            "--operations", "query", "--metrics", "accuracy", "--runs", "1",
            "--warmup-runs", "0"] + (["--comparator", comparator] if comparator else []))

    subprocess.run([args.binary, "flatten", raw, "--output", grid], check=True)
    reduce = subprocess.run(
        [args.binary, "atomic-costs", grid, "--output", table]
        + [arg for variant, metric in sorted(metrics.items())
           for arg in ("--accuracy-metric", f"{variant}={metric}")],
        capture_output=True, text=True)
    sys.stderr.write(reduce.stderr)
    if reduce.returncode != 0:
        sys.exit(f"atomic-costs failed (exit {reduce.returncode}); see above")
    # A skipped row is a failed measurement; a short table must not pass.
    lines = reduce.stderr.strip().splitlines()
    summary = lines[-1] if lines else ""
    expected = f"approxbench atomic-costs: {len(rows)} row(s), 0 skipped"
    if summary != expected:
        sys.exit(f"atomic-costs did not keep all {len(rows)} rows: {summary!r}")
    seed_mean_accuracy(args, table, rows, metrics)
    if hydra_datasets:
        with open(table) as f:
            entries = json.load(f)
        for entry in entries:
            at = entry["measured_at"]
            if entry["sketch"].startswith("hydra-"):
                at["dataset"] = hydra_datasets[(
                    entry["sketch"], int(entry["sketch_config"]["params"]["cols"]),
                    at["schema_width"])]
        with open(table, "w") as f:
            json.dump(entries, f, indent=2)
    print(f"Done. {table}", file=sys.stderr)
    return 0


def seed_mean_accuracy(args, table, rows, metrics):
    """Rewrite each sketch row's accuracy in `table` as the mean over seeds
    1..--seeds of one accuracy run at COST_N: the run the curves make at that
    N. Top-k at heaps above k keep the base's: curves are measured at k."""
    def params(config):
        return {k: float(v) for k, v in (kv.split("=") for kv in config.split())}

    means = {}
    for variant, library, config, comparator, data in rows:
        # Hydra rows keep their one pass: the optimizer reads Hydra's accuracy
        # from hydra_saturation.csv.
        if library == "exact" or variant.startswith("hydra-") or \
                float(params(config[1]).get("heap", TOPK_HEAPS[0])) != TOPK_HEAPS[0]:
            continue
        errors = [error(run(args.binary, [
            "--variant", variant, "--library", library, *config,
            "--operations", "query", "--metrics", "accuracy", "--comparator", comparator,
            "--runs", "1", "--warmup-runs", "0", "--seed", str(seed)] + data),
            metrics[variant]) for seed in range(1, args.seeds + 1)]
        base = {k: v for k, v in params(config[1]).items() if k != "heap"}
        means[(variant, tuple(sorted(base.items())))] = sum(errors) / len(errors)
    with open(table) as f:
        entries = json.load(f)
    for entry in entries:
        p = {k: float(v) for k, v in entry["sketch_config"]["params"].items() if k != "heap"}
        mean = means.get((entry["sketch"], tuple(sorted(p.items()))))
        if mean is not None:
            entry["query_accuracy"][entry["accuracy_metric"]] = mean
    with open(table, "w") as f:
        json.dump(entries, f, indent=2)


def hydra_labels(spec_text):
    """The label names of a Hydra datagen spec: its column_label before the
    value column, in schema order."""
    match = re.search(r"^column_label:\s*\[(.*)\]", spec_text, re.M)
    return [name.strip() for name in match.group(1).split(",")][:-1]


def hydra_spec(spec_text, records, seed):
    """`spec_text` at `records` rows, every distribution seed offset by
    1000 · (seed − 1), so seed 1 is the file as written and each study seed
    draws every column afresh. Plain text edits: the specs are this repo's."""
    text = re.sub(r"^row_num:.*$", f"row_num: {records}", spec_text, flags=re.M)
    return re.sub(r"^(\s*seed:\s*)(\d+)\s*$",
                  lambda m: f"{m.group(1)}{int(m.group(2)) + 1000 * (seed - 1)}", text,
                  flags=re.M)


def hydra_groupings(labels):
    """Every non-empty subset of `labels`, in schema order, smallest first."""
    return [list(c) for n in range(1, len(labels) + 1)
            for c in itertools.combinations(labels, n)]


def hydra_config(variant, w):
    return " ".join(filter(None, [f"rows={HYDRA_R} cols={w}", HYDRA_CELLS[variant]]))


def hydra_jobs(sweeps, specs, records=None):
    """Every run of `sweeps`, in one fixed order (so `--hydra-shard i/n` picks
    the same runs on every node): (dataset, dtype, variant, config, W,
    grouping, records, merge shards, seed). `specs` maps a yaml stem to its
    text; `records`, if given, replaces each sweep's record counts."""
    jobs = []
    for sweep in sweeps:
        datasets, ws, ns, shards, seeds = HYDRA_SWEEPS[sweep]
        ns = records or ns
        for stem, dtype, variants in datasets:
            groupings = hydra_groupings(hydra_labels(specs[stem]))
            for variant, w, grouping, n, m, seed in itertools.product(
                    variants, ws, groupings, ns, shards, range(1, seeds + 1)):
                jobs.append((stem, dtype, variant, hydra_config(variant, w), w, grouping, n,
                             m, seed))
    return jobs


def hydra_mem_bytes(job):
    """A run's estimated peak memory: its m shard grids plus the data."""
    _, _, variant, _, w, _, n, m, _ = job
    cell = HYDRA_CELL_BYTES.get(variant, HYDRA_UNIVMON_CELL_BYTES)
    return m * HYDRA_R * w * cell + n * HYDRA_RECORD_BYTES


def read_per_group(path):
    """[(n_q, error or None)] from a --per-group-out CSV; an empty error is a
    group the comparator could not score."""
    with open(path, newline="") as f:
        return [(int(r["n_q"]), float(r["error"]) if r["error"] else None)
                for r in csv.DictReader(f)]


def covered_max(groups, records, tau):
    """The worst error over the groups holding >= `tau` of the `records`,
    plus always the largest group (so no coverage is empty): the eval's
    covers_share rule. Unscored groups count for "largest" but have no
    error; "" when no covered group was scored."""
    if not groups:
        return ""
    largest = max(range(len(groups)), key=lambda i: groups[i][0])
    errors = [e for i, (n_q, e) in enumerate(groups)
              if e is not None and (i == largest or n_q / records >= tau)]
    return max(errors) if errors else ""


def hydra_row(job, labels, record, groups):
    """One hydra_saturation.csv row from a run's record and per-group file."""
    stem, _, variant, config, w, grouping, n, m, seed = job
    acc = record["bench"]["accuracy"]
    records = int(acc["records"])
    return {
        "variant": variant, "statistic": HYDRA_STATISTICS[variant], "config": config,
        "R": HYDRA_R, "W": w, "dataset": stem, "group_columns": ",".join(grouping),
        "schema_width": int(acc["schema_width"]), "records": records,
        "fanned_mass": int(acc["fanned_mass"]), "merge_shards": m,
        "merge_split": record["bench"].get("merge_split", ""), "seed": seed,
        **{k: acc.get(k, "") for k in ("err_mean", "err_p50", "err_p90", "err_max")},
        "groups_scored": int(acc["groups_scored"]),
        **{f"err_max_cov_{tau}": covered_max(groups, records, tau) for tau in HYDRA_COVERS},
    }


# hydra_saturation.csv's integer columns; the error columns are numbers or
# "" (no scored group), merge_split is "" for a query run, the rest are text.
HYDRA_INT_COLUMNS = ["R", "W", "schema_width", "records", "fanned_mass", "merge_shards",
                     "seed", "groups_scored"]


def hydra_row_parses(row):
    """Whether every column of a hydra_saturation.csv row holds a value of
    its type (a row cut off mid-write does not)."""
    try:
        for c in HYDRA_COLUMNS:
            if row.get(c) is None:
                return False
            if c in HYDRA_INT_COLUMNS:
                int(row[c])
            elif c.startswith("err_"):
                if row[c] != "":
                    float(row[c])
            elif c != "merge_split" and not row[c]:
                return False
    except ValueError:
        return False
    return True


def hydra_key(row):
    """A hydra_saturation.csv row's identity, from a row dict (values as the
    CSV holds them)."""
    return tuple(str(row[c]) for c in HYDRA_KEY)


class MemoryBudget:
    """Admits a run while the running runs' estimates fit in `total` bytes;
    a run larger than the whole budget runs alone."""

    def __init__(self, total):
        self.total, self.used, self.cond = total, 0, threading.Condition()

    def acquire(self, n):
        with self.cond:
            self.cond.wait_for(lambda: self.used == 0 or self.used + n <= self.total)
            self.used += n

    def release(self, n):
        with self.cond:
            self.used -= n
            self.cond.notify_all()


def hydra(args):
    """--phase hydra: every run of --hydra-sweeps (this node's share with
    --hydra-shard i/n), one row each in --out/hydra_saturation.csv, rows
    written as runs finish. --resume keeps the rows already there and runs
    the rest; a failed run is reported and the phase exits 1 at the end."""
    os.environ.setdefault("BENCH_WARMUP_SECS", "0")
    # --jobs runs share the cores; polars' own pool would oversubscribe them.
    os.environ.setdefault("POLARS_MAX_THREADS", "1")
    os.makedirs(args.out, exist_ok=True)
    specs = {}
    for sweep in args.hydra_sweeps.split(","):
        for stem, _, _ in HYDRA_SWEEPS[sweep][0]:
            with open(os.path.join(args.datagen, f"{stem}.yaml")) as f:
                specs[stem] = f.read()
    ns = [int(float(n)) for n in args.hydra_ns.split(",")] if args.hydra_ns else None
    jobs = hydra_jobs(args.hydra_sweeps.split(","), specs, ns)
    ws = {int(w) for w in args.hydra_ws.split(",")} if args.hydra_ws else None
    groupings = ({tuple(g.split(",")) for g in args.hydra_groupings.split(";")}
                 if args.hydra_groupings else None)
    jobs = [j for j in jobs
            if (ws is None or j[4] in ws)
            and (groupings is None or tuple(j[5]) in groupings)
            and j[8] <= (args.hydra_seeds or j[8])
            and (not args.hydra_variants or j[2] in args.hydra_variants.split(","))
            and (not args.hydra_datasets or j[0] in args.hydra_datasets.split(","))]
    shard, n_shards = (int(x) for x in args.hydra_shard.split("/"))
    jobs = jobs[shard::n_shards]

    table = os.path.join(args.out, HYDRA_TABLE)
    kept = []
    if args.resume and os.path.exists(table):
        with open(table, newline="") as f:
            text = f.read()
        # A row cut off mid-write (no line end, or a field that does not
        # parse) reruns.
        kept = [r for r in csv.DictReader(io.StringIO(text[:text.rfind("\n") + 1]))
                if hydra_row_parses(r)]
    done = {hydra_key(r) for r in kept}
    todo = [j for j in jobs if hydra_key({
        "variant": j[2], "config": j[3], "dataset": j[0], "group_columns": ",".join(j[5]),
        "records": j[6], "merge_shards": j[7], "seed": j[8]}) not in done]
    # Largest first, so the long runs do not trail the pool.
    todo.sort(key=lambda j: (j[6], j[7], j[4]), reverse=True)
    print(f"{len(jobs)} runs in shard {shard}/{n_shards}, {len(jobs) - len(todo)} kept, "
          f"{len(todo)} to run", file=sys.stderr)

    spec_dir = os.path.join(args.out, "hydra_specs")
    group_dir = os.path.join(args.out, "hydra_groups")
    os.makedirs(spec_dir, exist_ok=True)
    os.makedirs(group_dir, exist_ok=True)
    for stem, n, seed in {(j[0], j[6], j[8]) for j in todo}:
        with open(os.path.join(spec_dir, f"{stem}_n{n}_s{seed}.yaml"), "w") as f:
            f.write(hydra_spec(specs[stem], n, seed))
    budget = MemoryBudget(args.hydra_mem_gb * 1e9)

    def measure(i, job):
        stem, dtype, variant, config, _, grouping, n, m, seed = job
        labels = hydra_labels(specs[stem])
        per_group = os.path.join(group_dir, f"{shard}_{i}.csv")
        operation = ["query"] if m == 1 else [
            "merge", "--merge-shards", str(m), "--merge-split", "interleaved"]
        need = hydra_mem_bytes(job)
        budget.acquire(need)
        try:
            record = run(args.binary, [
                "--variant", variant, "--library", "lib", "--config", config,
                "--spec", os.path.join(spec_dir, f"{stem}_n{n}_s{seed}.yaml"),
                "--dtype", dtype,
                "--group-columns", ",".join(str(labels.index(g)) for g in grouping),
                "--per-group-out", per_group, "--operations", *operation,
                "--metrics", "accuracy", "--runs", "1", "--warmup-runs", "0"])
        finally:
            budget.release(need)
        groups = read_per_group(per_group)
        os.remove(per_group)
        return record, hydra_row(job, labels, record, groups)

    # The kept rows go to a new file that then replaces the table, so a
    # crash mid-rewrite cannot lose them; new rows append to it.
    with open(table + ".tmp", "w", newline="") as f, \
            open(os.path.join(args.out, "hydra_accuracy.jsonl"), "a") as raw:
        writer = csv.DictWriter(f, HYDRA_COLUMNS)
        writer.writeheader()
        writer.writerows(kept)
        f.flush()
        os.fsync(f.fileno())
        os.replace(table + ".tmp", table)
        failed = 0
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = {pool.submit(measure, i, j): j for i, j in enumerate(todo)}
            for k, future in enumerate(as_completed(futures), 1):
                job = futures[future]
                try:
                    record, row = future.result()
                except (subprocess.CalledProcessError, OSError, KeyError, ValueError) as e:
                    failed += 1
                    detail = getattr(e, "stderr", "") or e
                    print(f"  FAILED {job}: {str(detail).strip()[-500:]}", file=sys.stderr)
                    continue
                raw.write(json.dumps(record) + "\n")
                writer.writerow(row)
                f.flush()
                if k % 50 == 0 or k == len(todo):
                    print(f"  {k}/{len(todo)} runs", file=sys.stderr)
    print(f"Done. {table}", file=sys.stderr)
    if failed:
        print(f"{failed} run(s) failed; --resume reruns them", file=sys.stderr)
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--binary", default="./target/release/approxbench")
    parser.add_argument("--out", default="out")
    parser.add_argument("--families", default="frequency,topk,cardinality,quantile,hydra",
                        help="hydra: --phase optimizer-cost's Hydra rows")
    parser.add_argument("--one-config", action="store_true",
                        help="only the first config of each sketch")
    parser.add_argument("--thetas", default="0,0.5,0.8,1.0,1.2,1.5,2.0")
    parser.add_argument("--cardinalities", default="1000,100000,10000000")
    parser.add_argument("--alphas", default="1.1,1.5,2,3")
    parser.add_argument("--topk-ks", default=",".join(map(str, TOPK_KS)),
                        help="the k values top-k curves are graded at")
    parser.add_argument("--cost-shape", action=argparse.BooleanOptionalAction, default=True,
                        help="also run every config at --phase optimizer-cost's shape "
                             "(COST_*), so the cost table is a point on a curve")
    parser.add_argument("--n-min", type=float, default=1e3)
    parser.add_argument("--n-max", type=float, default=1e8)
    parser.add_argument("--per-decade", type=int, default=4)
    parser.add_argument("--seeds", type=int, default=5)
    parser.add_argument("--tolerance", type=float, default=0.10)
    parser.add_argument("--plateau-tail", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=1,
                        help="parallel accuracy runs (cost runs are always serial)")
    parser.add_argument("--phase",
                        choices=["accuracy", "cost", "crossover", "all", "optimizer-cost",
                                 "hydra"],
                        default="all",
                        help="cost reads saturation_curve.csv from --out; crossover "
                             "recomputes crossover.csv from the cost phase's JSONL in --out; "
                             "optimizer-cost writes the optimizer's cost table to --out "
                             "(the grid's configs at one shape, no curves); hydra writes "
                             "hydra_saturation.csv (HYDRA_SWEEPS, its own grid)")
    parser.add_argument("--points-from",
                        help="CSV with sketch,config,dist,param,cardinality columns "
                             "(e.g. a filtered saturation.csv): run only those points")
    parser.add_argument("--resume", action="store_true",
                        help="accuracy: keep the points whose curve over these sizes is "
                             "already in --out's saturation_curve.csv, run the rest")
    parser.add_argument("--cost-rows", type=int,
                        help="cost: measure only this rows= value of the Vector2D sketches "
                             "(other rows get blank cost columns and no crossover row)")
    parser.add_argument("--merge-shards-list", default="",
                        help="accuracy: e.g. 1,4,16,64: also score the sketch merged from "
                             "m shards, for each m>1, into saturation_merge_curve.csv")
    parser.add_argument("--datagen", default=HYDRA_DATAGEN,
                        help="hydra: where the sweeps' datagen specs are")
    parser.add_argument("--hydra-sweeps", default="eval,generic",
                        help="hydra: which of HYDRA_SWEEPS to run")
    parser.add_argument("--hydra-shard", default="0/1",
                        help="hydra: i/n runs every n-th run from the i-th (0-based), "
                             "to split one sweep across n machines")
    parser.add_argument("--hydra-mem-gb", type=float, default=200,
                        help="hydra: start a run only while the running runs' estimated "
                             "memory fits")
    # Narrow the sweeps (smoke runs, reruns); each keeps only what it names.
    parser.add_argument("--hydra-ns", help="hydra: these record counts instead")
    parser.add_argument("--hydra-ws", help="hydra: only these grid widths")
    parser.add_argument("--hydra-seeds", type=int, help="hydra: only seeds 1..this")
    parser.add_argument("--hydra-variants", help="hydra: only these variants")
    parser.add_argument("--hydra-datasets", help="hydra: only these yaml stems")
    parser.add_argument("--hydra-groupings",
                        help="hydra: only these groupings, ';'-separated, labels by name "
                             "joined by ',' in schema order")
    args = parser.parse_args()
    if args.phase == "hydra":
        return hydra(args)

    families = args.families.split(",")
    thetas = [float(t) for t in args.thetas.split(",")]
    cardinalities = [int(k) for k in args.cardinalities.split(",")]
    alphas = [float(a) for a in args.alphas.split(",")]
    topk_ks = sorted({int(k) for k in args.topk_ks.split(",") if k})
    if TOPK_K not in topk_ks:
        sys.exit(f"--topk-ks must include {TOPK_K}, the k the cost table's top-k rows "
                 "are graded at")
    ns = checkpoints(args.n_min, args.n_max, args.per_decade)
    seeds = list(range(1, args.seeds + 1))
    shard_list = [int(m) for m in args.merge_shards_list.split(",") if m]
    if args.phase == "optimizer-cost":
        return optimizer_cost(args, families)
    os.environ.setdefault("BENCH_WARMUP_SECS", "0")
    os.makedirs(args.out, exist_ok=True)

    # A point is (family, variant, config, comparator, metric, dataset args,
    # dist, param, cardinality).
    points = []
    for family, variant, configs, comparator, metric in SKETCHES:
        if family not in families:
            continue
        configs = configs[:1] if args.one_config else configs
        if family == "topk":
            configs = [c + ("" if k == TOPK_K else f" topk_k={k}")
                       for c in configs for k in topk_ks]
        for config in configs:
            if family == "quantile":
                extra = args.cost_shape and COST_PARETO_ALPHA not in alphas
                for alpha in alphas + [COST_PARETO_ALPHA] * extra:
                    # --cardinality is required by the CLI but ignored for pareto.
                    dataset = ["--dataset", "pareto", "--pareto-alpha", str(alpha),
                               "--pareto-scale", "1000", "--cardinality", "1",
                               "--dtype", "i64"]
                    points.append((family, variant, config, comparator, metric,
                                   dataset, "pareto", alpha, ""))
                continue
            # Plus the cost table's shape as a full row and column (θ = 1.1 at
            # every K, K = 1e4 at every θ), so its rows are points on a curve
            # and the grid stays a full cross for the optimizer's bracketing.
            grid_thetas = thetas + [COST_THETA] * (args.cost_shape and COST_THETA not in thetas)
            grid_keys = cardinalities + [COST_KEYS] * (
                args.cost_shape and COST_KEYS not in cardinalities)
            for theta, k in [(t, k) for t in grid_thetas for k in grid_keys]:
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

    def accuracy(point, n, seed, shards=1):
        _, variant, config, comparator, metric, dataset, *_ = point
        # One shard is the plain query; more score the sketch merged from them.
        operation = ["query"] if shards == 1 else ["merge", "--merge-shards", str(shards)]
        return run(args.binary, [
            "--variant", variant, "--library", "lib", "--config", config,
            "--operations", *operation, "--metrics", "accuracy",
            "--comparator", comparator, "--runs", "1", "--warmup-runs", "0",
            "--size", str(n), "--seed", str(seed),
        ] + dataset)

    if args.phase in ("cost", "crossover"):
        results = read_curve(os.path.join(args.out, "saturation_curve.csv"), points, ns,
                             args.tolerance, args.plateau_tail)
    else:
        curve_path = os.path.join(args.out, "saturation_curve.csv")
        # --resume keeps complete curves and writes them back first, so a
        # point cut off mid-write is dropped and rerun, and a second
        # interruption loses nothing that was kept. Rows of points outside
        # this grid are written back as they are, so a narrower resume
        # deletes nothing. With --merge-shards-list a point is complete only
        # once its merge curve is too.
        merge_path = os.path.join(args.out, "saturation_merge_curve.csv")
        done, kept_rows, kept_merge_rows = {}, [], []
        if args.resume and os.path.exists(curve_path):
            grid = {point_key(p[1], p[2], *p[6:]) for p in points}
            done = {key: c for key, c in load_curves(curve_path).items()
                    if key in grid and [n for n, _, _ in c] == ns}
            if shard_list:
                merge_done = (load_curves(merge_path, shards=True)
                              if os.path.exists(merge_path) else {})
                done = {key: c for key, c in done.items()
                        if all([n for n, _, _ in merge_done.get(key + (m,), [])] == ns
                               for m in shard_list)}
                if os.path.exists(merge_path):
                    with open(merge_path, newline="") as f:
                        for r in csv.DictReader(f):
                            key = point_key(r["sketch"], r["config"], r["dist"], r["param"],
                                            r["cardinality"])
                            if key not in grid or (key in done and int(r["shards"]) in shard_list):
                                kept_merge_rows.append([r[c] for c in MERGE_CURVE_COLUMNS])
            with open(curve_path, newline="") as f:
                for r in csv.DictReader(f):
                    key = point_key(r["sketch"], r["config"], r["dist"], r["param"],
                                    r["cardinality"])
                    if key not in grid or key in done:
                        kept_rows.append([r[c] for c in CURVE_COLUMNS])
        raw = open(os.path.join(args.out, "saturation_accuracy.jsonl"),
                   "a" if args.resume else "w")
        curve_file = open(curve_path, "w", newline="")
        curve = csv.writer(curve_file)
        curve.writerow(CURVE_COLUMNS)
        curve.writerows(kept_rows)
        curve_file.flush()
        if shard_list:
            merge_file = open(merge_path, "w", newline="")
            merge_curve = csv.writer(merge_file)
            merge_curve.writerow(MERGE_CURVE_COLUMNS)
            merge_curve.writerows(kept_merge_rows)
            merge_file.flush()
        # Submit everything up front so the pool stays full; results are consumed
        # in point order, so the curve CSV fills point by point.
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [
                None if point_key(p[1], p[2], *p[6:]) in done else
                [[pool.submit(accuracy, p, n, s) for s in seeds] for n in ns] for p in points
            ]
            merge_futures = [
                None if per_n is None else
                [{m: [pool.submit(accuracy, p, n, s, m) for s in seeds]
                  for m in shard_list if m > 1} for n in ns]
                for p, per_n in zip(points, futures)
            ]
            print(f"{len(done)} points kept, {sum(f is not None for f in futures)} to run",
                  file=sys.stderr)
            results = []
            try:
                for point, per_n, merge_per_n in zip(points, futures, merge_futures):
                    family, variant, config, _, metric, _, dist, param, k = point
                    if per_n is None:
                        kept = done[point_key(variant, config, dist, param, k)]
                        means = [mean for _, mean, _ in kept]
                        n_sat = n_saturation(ns, means, args.tolerance, args.plateau_tail,
                                             [se for _, _, se in kept])
                        results.append((point, n_sat, means[-1]))
                        continue
                    means, ses = [], []
                    for n, per_seed, per_shards in zip(ns, per_n, merge_per_n):
                        records = [f.result() for f in per_seed]
                        for r in records:
                            raw.write(json.dumps(r) + "\n")
                        mean, se = mean_and_se([error(r, metric) for r in records])
                        means.append(mean)
                        ses.append(se)
                        curve.writerow([family, variant, config, dist, param, k, n, mean, se])
                        for m in shard_list:
                            if m == 1:
                                mean, se = means[-1], ses[-1]
                            else:
                                merged = [f.result() for f in per_shards[m]]
                                for r in merged:
                                    raw.write(json.dumps(r) + "\n")
                                mean, se = mean_and_se([error(r, metric) for r in merged])
                            merge_curve.writerow(
                                [family, variant, config, dist, param, k, n, m, mean, se])
                    curve_file.flush()
                    if shard_list:
                        merge_file.flush()
                    n_sat = n_saturation(ns, means, args.tolerance, args.plateau_tail, ses)
                    results.append((point, n_sat, means[-1]))
                    print(f"  {variant} ({config}) {dist}={param} K={k}: n_sat={n_sat}",
                          file=sys.stderr)
            except BaseException:
                # A failed run (or Ctrl-C) should surface now, not after the queue drains.
                for f in (f for per_n in futures if per_n for fs in per_n for f in fs):
                    f.cancel()
                for f in (f for per_n in merge_futures if per_n
                          for by_m in per_n for fs in by_m.values() for f in fs):
                    f.cancel()
                raise
        raw.close()
        curve_file.close()
        if shard_list:
            merge_file.close()

    # Cost at the final N, one serial run per point with the merge setup of
    # --phase optimizer-cost. --phase accuracy leaves these blank.
    rows, records = [], []
    cost = args.phase != "accuracy"
    rebuild = args.phase == "crossover"
    cost_path = os.path.join(args.out, "saturation_cost.jsonl")
    saved = load_saved(cost_path, point_key) if rebuild else None
    cost_raw = open(cost_path, "w") if cost and not rebuild else None
    for point, n_sat, final_error in results:
        family, variant, config, _, metric, dataset, dist, param, k = point
        record = {}
        # rows=5 costs ~5/3 of rows=3 (insert, query and memory are per row).
        skip = args.cost_rows is not None and config.startswith("rows=") \
            and not config.startswith(f"rows={args.cost_rows} ")
        if rebuild and not skip:
            record = saved_record(saved, point_key(variant, config, dist, param, k),
                                  cost_path)
        elif cost and not skip:
            record = run(args.binary, [
                "--variant", variant, "--library", "lib", "--config", config,
                "--operations", "insert,query,merge",
                "--metrics", "throughput,cpu,memory",
                "--merge-shards", str(MERGE_SHARDS), "--runs", "3", "--warmup-runs", "1",
                "--size", str(ns[-1]), "--seed", str(seeds[0]), "--flat",
            ] + dataset)
            cost_raw.write(json.dumps(
                {"key": [variant, config, dist, param, k], **record}) + "\n")
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

    if not rebuild:
        summary_path = os.path.join(args.out, "saturation.csv")
        # --resume over a narrower grid keeps the other points' rows, as the
        # curve files keep their curves.
        kept_summary = []
        # Accuracy phase only: a cost phase truncates saturation_cost.jsonl,
        # so other points' cost columns would outlive their records.
        if args.resume and args.phase == "accuracy" and os.path.exists(summary_path):
            grid = {point_key(p[1], p[2], *p[6:]) for p in points}
            with open(summary_path, newline="") as f:
                kept_summary = [[r.get(c, "") for c in SUMMARY_COLUMNS]
                                for r in csv.DictReader(f)
                                if point_key(r["sketch"], r["config"], r["dist"], r["param"],
                                             r["cardinality"]) not in grid]
        with open(summary_path, "w", newline="") as f:
            writer = csv.writer(f)
            writer.writerow(SUMMARY_COLUMNS)
            writer.writerows(kept_summary + rows)
        print(f"Done. {os.path.join(args.out, 'saturation.csv')}", file=sys.stderr)
    if not cost:
        return 0

    # The family's exact baseline at every checkpoint, once per (baseline,
    # dataset). Memory is its memory_bytes after prepare (the buffered stream
    # plus any count table), the same self-reported formula as the sketch's;
    # CPU is insert + prepare (the polars pass) + one query.
    exact = {}
    measured = [(r, record) for r, record in zip(results, records) if record]
    exact_path = os.path.join(args.out, "exact_cost.jsonl")
    saved_exact = load_saved(exact_path, lambda v, ds, n, op: (v, tuple(ds), n, op)) \
        if rebuild else None
    with open(exact_path, "a" if rebuild else "w") as exact_raw:
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
                if rebuild:
                    timed, prepared = (saved_record(saved_exact, key + (op,), exact_path)
                                       for op in ("timed", "prepared"))
                else:
                    timed = run(args.binary, base + [
                        "--operations", "insert,query", "--metrics", "throughput,cpu,memory"])
                    prepared = run(args.binary, base + [
                        "--operations", "prepare", "--metrics", "latency,cpu,memory"])
                    for op, r in (("timed", timed), ("prepared", prepared)):
                        exact_raw.write(json.dumps(
                            {"key": [variant, point[5], n, op], **r}) + "\n")
                exact[key] = (prepared["memory_bytes"], cpu_secs(timed, "insert")
                              + query_secs(timed) + cpu_secs(prepared, "prepare"))

    with open(os.path.join(args.out, "crossover.csv"), "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(CROSSOVER_COLUMNS)
        for (point, n_sat, _), record in measured:
            family, variant, config, _, _, dataset, dist, param, k = point
            per_n = [exact[(EXACT[family][0], tuple(dataset), n)] for n in ns]
            insert, query = cpu_secs(record, "insert"), query_secs(record)
            memory = record["memory_bytes"]
            # Measured at the final N only: insert CPU is a per-item cost, so it
            # scales with N; one query's CPU and memory are taken as constant.
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
