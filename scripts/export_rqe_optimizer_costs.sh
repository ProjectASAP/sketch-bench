#!/usr/bin/env bash
# Measure the small atomic-cost grid consumed by rqe-optimizer.
#
# This is intentionally separate from export_atomic_costs.sh: that script
# serves ASAPQuery's pinned grid, while this one covers the variants named by
# rqe_optimizer::Capability::families(). Only those also in
# DEPLOYABLE_FAMILIES become candidates; the rest are measured for when the
# engine gains them, and for research baselines.
#
# Usage:
#   scripts/export_rqe_optimizer_costs.sh OUT_DIR
#   scripts/export_rqe_optimizer_costs.sh OUT_DIR --no-build
#
# OUT_DIR must not exist yet, so a run never overwrites earlier measurements.

set -euo pipefail

BUILD=1
OUT_DIR=
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=0 ;;
        -*) echo "ERROR: unknown flag $arg" >&2; exit 1 ;;
        *)
            [[ -z "$OUT_DIR" ]] || { echo "ERROR: more than one OUT_DIR given" >&2; exit 1; }
            OUT_DIR=$arg
            ;;
    esac
done
if [[ -z "$OUT_DIR" ]]; then
    echo "Usage: $0 OUT_DIR [--no-build]" >&2
    exit 1
fi
if [[ -e "$OUT_DIR" ]]; then
    echo "ERROR: $OUT_DIR already exists; pick a new directory." >&2
    exit 1
fi

RAW_JSONL="$OUT_DIR/rqe_atomic_costs_raw.jsonl"
GRID_JSONL="$OUT_DIR/rqe_atomic_costs_grid.jsonl"
MERGE_ACCURACY_JSONL="$OUT_DIR/rqe_merge_accuracy_grid.jsonl"
TABLE_JSON="$OUT_DIR/rqe_atomic_costs.json"
mkdir -p "$OUT_DIR"

if [[ $BUILD -eq 1 ]]; then
    echo "==> Building approxbench (release)..." >&2
    cargo build -p aqpbm-cli --release
fi

BINARY=./target/release/approxbench
if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: $BINARY not found. Run without --no-build." >&2
    exit 1
fi

RUNS=5
WARMUP=3
SIZE=1000000
CARDINALITY=100000
SEED=42
MERGE_SHARDS=16
# Accuracy after folding m shards, for the optimizer's L/x windows per query.
# One shard is the accuracy pass's single instance.
MERGE_ACCURACY_SHARDS=(4 16 64)

# The swept parameters, every one of them. Exact accumulators take none.
HLL_PRECISIONS=(12 14)
KLL_KS=(200 500)
DD_ALPHAS=(0.01 0.02)
CMS_HEAP_ROWS=(3 5)
CMS_HEAP_COLS=(2048)
UNIVMON_CONFIGS=(
    "heap_size=1000 sketch_row=5 sketch_col=2048 layer_size=8"
    "heap_size=500 sketch_row=3 sketch_col=1024 layer_size=6"
)
HYDRA_KLL_CONFIGS=(
    "rows=3 cols=128 cell_k=200"
    "rows=3 cols=256 cell_k=200"
    "rows=3 cols=512 cell_k=200"
    "rows=5 cols=256 cell_k=200"
    "rows=3 cols=256 cell_k=500"
)

# One row: a cost pass, an accuracy pass, then one merge-accuracy pass per
# merge count, over the same data. Arguments
# after the comparator name the data (`--dataset ...` or `--spec ...`, plus
# `--dtype`). An empty config omits `--config` (the exact accumulators).
measure() {
    local variant=$1 library=$2 config=$3 comparator=$4
    shift 4
    local config_args=()
    [[ -n "$config" ]] && config_args=(--config "$config")
    echo "  $variant${config:+ ($config)}" >&2

    # Cost goes first. Secondary query timings accompany both passes, and
    # flattening retains the first one; this preserves the five-sample cost
    # timings that atomic-costs pairs with query throughput.
    "$BINARY" sketchbench \
        --variant "$variant" --library "$library" "${config_args[@]}" \
        --operations insert,query,merge --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" --runs "$RUNS" --warmup-runs "$WARMUP" \
        "$@" --seed "$SEED" --report "$RAW_JSONL"
    "$BINARY" sketchbench \
        --variant "$variant" --library "$library" "${config_args[@]}" \
        --operations query --metrics accuracy --comparator "$comparator" \
        --runs "$RUNS" --warmup-runs "$WARMUP" \
        "$@" --seed "$SEED" --report "$RAW_JSONL"
    # One `--flat` row per merge count: a cell's counts would collide in
    # `flatten`, which keeps one merge slot per cell. atomic-costs joins them.
    local m
    for m in "${MERGE_ACCURACY_SHARDS[@]}"; do
        "$BINARY" sketchbench \
            --variant "$variant" --library "$library" "${config_args[@]}" \
            --operations merge --metrics accuracy --comparator "$comparator" \
            --merge-shards "$m" \
            "$@" --seed "$SEED" --flat --report "$MERGE_ACCURACY_JSONL"
    done
}

# One-column sketches on inline data. Zipf is deliberate. Uniform data makes
# request-rate relative error meaningless in the rare-key tail and makes top-k
# accuracy degenerate.
point() {
    local variant=$1 config=$2 comparator=$3
    measure "$variant" lib "$config" "$comparator" \
        --dataset zipf --zipf-s 1.1 --size "$SIZE" --cardinality "$CARDINALITY" \
        --dtype i64
}

# Rows that need a multi-column spec: grouped records for the exact
# accumulators and Hydra, key/value pairs for UnivMon cardinality.
point_spec() {
    local variant=$1 library=$2 config=$3 comparator=$4 spec=$5 dtype=$6
    measure "$variant" "$library" "$config" "$comparator" --spec "$spec" --dtype "$dtype"
}

# Exact accumulators live under library "exact" and take no config. Increase
# reads counters; the rest read grouped i64 records.
echo "==> Exact accumulators: sum, min, max, increase, delta set" >&2
point_spec exact-sum exact "" sum-or-count configs/datagen/hydra_columns.yaml i64
point_spec exact-min exact "" min configs/datagen/hydra_columns.yaml i64
point_spec exact-max exact "" max configs/datagen/hydra_columns.yaml i64
point_spec exact-increase exact "" rate-or-increase configs/datagen/counter_columns.yaml i64
point_spec exact-delta-set exact "" key-set configs/datagen/hydra_columns.yaml i64

# HydraKLL rows are whole-sketch: one instance serves every group, and memory
# and merge cost are not divided per group (`subpop-rank-error` reports no
# `groups_per_instance`). The optimizer prices a row as card(G) x per-instance
# cost, so these rows must not become candidates until #142 lands. Cost and
# accuracy hold at this spec's group count only. The spec gives each label its
# own alphabet, so rank error is the sketch's, not label collisions' (#74).
echo "==> Quantiles: KLL, DDSketch, HydraKLL" >&2
for k in "${KLL_KS[@]}"; do
    point kll-percall "k=$k" rank-error
done
for alpha in "${DD_ALPHAS[@]}"; do
    point dd "alpha=$alpha" rank-error
done
for config in "${HYDRA_KLL_CONFIGS[@]}"; do
    point_spec hydra-kll lib "$config" subpop-rank-error \
        configs/datagen/hydra_kll_disjoint_labels.yaml f64
done

echo "==> Cardinality: HLL and UnivMon" >&2
for lg_k in "${HLL_PRECISIONS[@]}"; do
    point hll "lg_k=$lg_k" cardinality
done
for config in "${UNIVMON_CONFIGS[@]}"; do
    point_spec univmon-cardinality lib "$config" keyed-cardinality \
        configs/datagen/univmon_columns.yaml u64
done

echo "==> Top-k: CMS heap, CountSketch heap, UnivMon" >&2
for rows in "${CMS_HEAP_ROWS[@]}"; do
    for cols in "${CMS_HEAP_COLS[@]}"; do
        point cms-heap-topk-fastpath-vector2d "rows=$rows cols=$cols" topk
        point countsketch-heap-topk-fastpath-vector2d "rows=$rows cols=$cols" topk
    done
done
for config in "${UNIVMON_CONFIGS[@]}"; do
    point univmon-topk "$config" topk
done

echo "==> Flattening cost + accuracy passes..." >&2
"$BINARY" flatten "$RAW_JSONL" --output "$GRID_JSONL"

echo "==> Reducing to atomic-cost table (expect 24 row(s), 0 skipped)..." >&2
# A skipped row is a failed measurement, e.g. an exact row that missed a group
# scores an infinite error, which JSON holds as null. Fail rather than publish
# a table that is silently short.
summary=$("$BINARY" atomic-costs "$GRID_JSONL" --merge-accuracy "$MERGE_ACCURACY_JSONL" \
    --output "$TABLE_JSON" 2>&1 | tee /dev/stderr | tail -n 1)
if [[ "$summary" != *", 0 skipped" ]]; then
    echo "ERROR: atomic-costs skipped rows; see the reasons above." >&2
    exit 1
fi

echo "Done. $TABLE_JSON" >&2
