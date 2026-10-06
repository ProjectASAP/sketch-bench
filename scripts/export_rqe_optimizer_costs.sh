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
#   scripts/export_rqe_optimizer_costs.sh
#   scripts/export_rqe_optimizer_costs.sh --no-build

set -euo pipefail

BUILD=1
if [[ "${1:-}" == "--no-build" ]]; then
    BUILD=0
fi

OUT_DIR=out
RAW_JSONL="$OUT_DIR/rqe_atomic_costs_raw.jsonl"
GRID_JSONL="$OUT_DIR/rqe_atomic_costs_grid.jsonl"
TABLE_JSON="$OUT_DIR/rqe_atomic_costs.json"
mkdir -p "$OUT_DIR"
rm -f "$RAW_JSONL" "$GRID_JSONL"

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

HLL_PRECISIONS=(12 14)
KLL_KS=(200 500)
DD_ALPHAS=(0.01 0.02)
CMS_HEAP_ROWS=(3 5)
CMS_HEAP_COLS=(2048)

# Zipf is deliberate. Uniform data makes request-rate relative error
# meaningless in the rare-key tail and makes top-k accuracy degenerate.
point() {
    local variant=$1 config=$2 comparator=$3
    echo "  $variant ($config)" >&2

    # Cost goes first. Secondary query timings accompany both passes, and
    # flattening retains the first one; this preserves the five-sample cost
    # timings that atomic-costs pairs with query throughput.
    "$BINARY" sketchbench \
        --variant "$variant" --library lib --config "$config" \
        --operations insert,query,merge --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" --runs "$RUNS" --warmup-runs "$WARMUP" \
        --dataset zipf --zipf-s 1.1 --size "$SIZE" --cardinality "$CARDINALITY" \
        --dtype i64 --seed "$SEED" --report "$RAW_JSONL"
    "$BINARY" sketchbench \
        --variant "$variant" --library lib --config "$config" \
        --operations query --metrics accuracy --comparator "$comparator" \
        --runs "$RUNS" --warmup-runs "$WARMUP" \
        --dataset zipf --zipf-s 1.1 --size "$SIZE" --cardinality "$CARDINALITY" \
        --dtype i64 --seed "$SEED" --report "$RAW_JSONL"
}

# Exact accumulators take no --config, live under library "exact", and ingest
# grouped records: label columns, then an i64 value. Increase reads counters.
point_exact() {
    local variant=$1 comparator=$2 spec=$3
    echo "  $variant" >&2
    "$BINARY" sketchbench \
        --variant "$variant" --library exact \
        --operations insert,query,merge --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" --runs "$RUNS" --warmup-runs "$WARMUP" \
        --spec "$spec" --dtype i64 --seed "$SEED" --report "$RAW_JSONL"
    "$BINARY" sketchbench \
        --variant "$variant" --library exact \
        --operations query --metrics accuracy --comparator "$comparator" \
        --runs "$RUNS" --warmup-runs "$WARMUP" \
        --spec "$spec" --dtype i64 --seed "$SEED" --report "$RAW_JSONL"
}

# UnivMon consumes key/value pairs. The inline dataset path is one-column, so
# use the tracked two-column spec whose key column is u64.
point_univmon() {
    local config=$1
    echo "  univmon-cardinality ($config)" >&2
    "$BINARY" sketchbench \
        --variant univmon-cardinality --library lib --config "$config" \
        --operations insert,query,merge --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" --runs "$RUNS" --warmup-runs "$WARMUP" \
        --spec configs/datagen/univmon_columns.yaml --dtype u64 --seed "$SEED" \
        --report "$RAW_JSONL"
    "$BINARY" sketchbench \
        --variant univmon-cardinality --library lib --config "$config" \
        --operations query --metrics accuracy --comparator keyed-cardinality \
        --runs "$RUNS" --warmup-runs "$WARMUP" \
        --spec configs/datagen/univmon_columns.yaml --dtype u64 --seed "$SEED" \
        --report "$RAW_JSONL"
}

echo "==> Exact accumulators: sum, min, max, increase" >&2
point_exact exact-sum sum-or-count configs/datagen/hydra_columns.yaml
point_exact exact-min min configs/datagen/hydra_columns.yaml
point_exact exact-max max configs/datagen/hydra_columns.yaml
point_exact exact-increase rate-or-increase configs/datagen/counter_columns.yaml

echo "==> Quantiles: KLL and DDSketch" >&2
for k in "${KLL_KS[@]}"; do
    point kll-percall "k=$k" rank-error
done
for alpha in "${DD_ALPHAS[@]}"; do
    point dd "alpha=$alpha" rank-error
done

echo "==> Cardinality: HLL and UnivMon" >&2
for lg_k in "${HLL_PRECISIONS[@]}"; do
    point hll "lg_k=$lg_k" cardinality
done
point_univmon "heap_size=1000 sketch_row=5 sketch_col=2048 layer_size=8"
point_univmon "heap_size=500 sketch_row=3 sketch_col=1024 layer_size=6"

echo "==> Top-k: CMS heap, CountSketch heap, UnivMon" >&2
for rows in "${CMS_HEAP_ROWS[@]}"; do
    for cols in "${CMS_HEAP_COLS[@]}"; do
        point cms-heap-topk-fastpath-vector2d "rows=$rows cols=$cols" topk
        point countsketch-heap-topk-fastpath-vector2d "rows=$rows cols=$cols" topk
    done
done
point univmon-topk "heap_size=1000 sketch_row=5 sketch_col=2048 layer_size=8" topk
point univmon-topk "heap_size=500 sketch_row=3 sketch_col=1024 layer_size=6" topk

echo "==> Flattening cost + accuracy passes..." >&2
"$BINARY" flatten "$RAW_JSONL" --output "$GRID_JSONL"

echo "==> Reducing to atomic-cost table (expect 18 row(s), 0 skipped)..." >&2
"$BINARY" atomic-costs "$GRID_JSONL" --output "$TABLE_JSON"

echo "Done. $TABLE_JSON" >&2
