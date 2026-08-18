#!/usr/bin/env bash
# export_atomic_costs.sh — measure ASAPQuery's optimizer grid and emit its
# atomic-cost table (ASAPQuery#524, sketch-bench#30).
#
# Loops exactly the param grid ASAPQuery's asap-planner-rs/src/optimizer/
# constants.rs sweeps (CMS_DEPTHS x CMS_WIDTHS, HLL_PRECISIONS, KLL_KS), one
# `sketchbench --flat` invocation per grid point, against the specific
# (algorithm, impl) pair each ASAPQuery accumulator actually deploys — not
# just any "lib" row for the family:
#   - cms-fastpath-vector2d: asap_sketchlib's `CountMinSketch` is a type alias
#     for `CountMin<Vector2D<f64>, FastPath>` (message_pack_format/portable/
#     countminsketch.rs), which is what count_min_sketch_accumulator.rs wraps.
#   - hll: hll_accumulator.rs builds `HllSketch::new(HllVariant::Regular, _)`,
#     matching the "lib" row's `HyperLogLog<Classic>` (not `hll-hip`).
#   - kll-percall: datasketches_kll_accumulator.rs calls `.quantile()` per
#     query, matching "kll-percall" (not "kll-cdf", which builds a CDF in
#     `prepare`).
# Hydra (subpopulation) families and CMS-with-heap are out of scope: greedy's
# cost functions don't consume per-param subpopulation costs yet (#525), and
# CountMinSketchWithHeap's memory is an analytic bound in ASAPQuery, not a
# sketch-bench lookup (#524) — sketch-bench has no wrapper for it at all.
#
# Usage:
#   scripts/export_atomic_costs.sh             # build + run everything
#   scripts/export_atomic_costs.sh --no-build   # skip cargo build
#
# Output: out/atomic_costs.json (the flat AtomicCostTable ASAPQuery loads),
# plus out/atomic_costs_grid.jsonl (the raw --flat MergedRecord rows, kept
# for provenance / re-deriving the table without re-running the benchmark).

set -euo pipefail

BUILD=1
if [[ "${1:-}" == "--no-build" ]]; then
    BUILD=0
fi

OUT_DIR=out
GRID_JSONL="$OUT_DIR/atomic_costs_grid.jsonl"
TABLE_JSON="$OUT_DIR/atomic_costs.json"
mkdir -p "$OUT_DIR"
rm -f "$GRID_JSONL"

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
# > MIN_MERGE_SHARDS(2): several folds dilute per-call timer overhead/noise,
# which single-fold runs are dominated by at this operation's sub-ms scale.
MERGE_SHARDS=16

point() {
    local algorithm=$1 config=$2
    echo "  $algorithm ($config)" >&2
    "$BINARY" sketchbench \
        --algorithm "$algorithm" \
        --impl lib \
        --config "$config" \
        --operations insert,query,merge \
        --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" \
        --runs "$RUNS" \
        --warmup-runs "$WARMUP" \
        --dataset uniform \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --seed "$SEED" \
        --flat \
        --report "$GRID_JSONL"
}

echo "==> cms-fastpath-vector2d (CMS_DEPTHS x CMS_WIDTHS)" >&2
for depth in 3 5; do
    for width in 512 1024 2048; do
        point cms-fastpath-vector2d "rows=$depth cols=$width"
    done
done

echo "==> hll (HLL_PRECISIONS)" >&2
for lg_k in 12 14; do
    point hll "lg_k=$lg_k"
done

echo "==> kll-percall (KLL_KS)" >&2
for k in 200 500; do
    point kll-percall "k=$k"
done

echo "==> Reducing to atomic-cost table..." >&2
"$BINARY" atomic-costs "$GRID_JSONL" --output "$TABLE_JSON"

echo "" >&2
echo "Done. $TABLE_JSON" >&2
