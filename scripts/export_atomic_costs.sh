#!/usr/bin/env bash
# export_atomic_costs.sh — measure ASAPQuery's optimizer grid and emit its
# atomic-cost table (ASAPQuery#524, sketch-bench#30).
#
# Loops exactly the param grid ASAPQuery's asap-planner-rs/src/optimizer/
# constants.rs sweeps (CMS_DEPTHS x CMS_WIDTHS, HLL_PRECISIONS, KLL_KS), two
# `sketchbench` invocations per grid point (cost, then accuracy — see `point`
# below for why they can't be one), against the specific (algorithm, impl)
# pair each ASAPQuery accumulator actually deploys — not just any "lib" row
# for the algorithm:
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
# Output: out/atomic_costs.json (the versioned AtomicCostDocument ASAPQuery
# loads — one profile per workload, entries grouped inside it), plus
# out/atomic_costs_grid.jsonl (the flattened MergedRecord rows, kept for
# provenance / re-deriving the document without re-running the benchmark) and
# out/atomic_costs_raw.jsonl (the un-flattened Records `flatten` folds into
# it — see below for why there are two passes per grid point).

set -euo pipefail

BUILD=1
if [[ "${1:-}" == "--no-build" ]]; then
    BUILD=0
fi

OUT_DIR=out
RAW_JSONL="$OUT_DIR/atomic_costs_raw.jsonl"
GRID_JSONL="$OUT_DIR/atomic_costs_grid.jsonl"
TABLE_JSON="$OUT_DIR/atomic_costs.json"
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
# > MIN_MERGE_SHARDS(2): several folds dilute per-call timer overhead/noise,
# which single-fold runs are dominated by at this operation's sub-ms scale.
MERGE_SHARDS=16

# Mirrors ASAPQuery's asap-planner-rs/src/optimizer/constants.rs grid — kept
# under these names so a diff against that file is a direct name-for-name
# comparison instead of a guess at which literal means what.
CMS_DEPTHS=(3 5)
CMS_WIDTHS=(512 1024 2048)
HLL_PRECISIONS=(12 14)
KLL_KS=(200 500)

# Two invocations, not one: `--metrics` is crossed with every named
# `--operations`, and accuracy only applies to `query` — asking for it
# alongside insert/merge throughput in one invocation is refused by name
# ("nothing measures the accuracy of insert"), unconditionally, regardless of
# comparator. Both write raw (non-`--flat`) records to the same file; `flatten`
# below folds the two passes of one grid point back into one row.
#
# Accuracy first, cost second. `flatten_record` gives every flattened field a
# canonical owning pass: accuracy contributes only query_accuracy, while the
# throughput pass contributes the timing/CPU/resource fields that atomic-costs
# pairs with its throughput samples. The output is therefore independent of
# this order; keeping the passes adjacent simply makes the raw provenance easy
# to read. Accuracy always runs exactly 1 sample (`aqpbm_core::runs_for`), so
# accepting its incidental timing would make the reducer correctly reject a
# mixed 1-sample/5-sample row.
point() {
    local algorithm=$1 config=$2 comparator=$3
    echo "  $algorithm ($config)" >&2
    "$BINARY" sketchbench \
        --variant "$algorithm" \
        --library lib \
        --config "$config" \
        --operations query \
        --metrics accuracy \
        --comparator "$comparator" \
        --runs "$RUNS" \
        --warmup-runs "$WARMUP" \
        --dataset uniform \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --dtype i64 \
        --seed "$SEED" \
        --report "$RAW_JSONL"
    "$BINARY" sketchbench \
        --variant "$algorithm" \
        --library lib \
        --config "$config" \
        --operations insert,query,merge \
        --metrics throughput,cpu,memory \
        --merge-shards "$MERGE_SHARDS" \
        --runs "$RUNS" \
        --warmup-runs "$WARMUP" \
        --dataset uniform \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --dtype i64 \
        --seed "$SEED" \
        --report "$RAW_JSONL"
}

echo "==> cms-fastpath-vector2d (CMS_DEPTHS x CMS_WIDTHS)" >&2
for depth in "${CMS_DEPTHS[@]}"; do
    for width in "${CMS_WIDTHS[@]}"; do
        point cms-fastpath-vector2d "rows=$depth cols=$width" frequency
    done
done

echo "==> hll (HLL_PRECISIONS)" >&2
for lg_k in "${HLL_PRECISIONS[@]}"; do
    point hll "lg_k=$lg_k" cardinality
done

echo "==> kll-percall (KLL_KS)" >&2
for k in "${KLL_KS[@]}"; do
    point kll-percall "k=$k" rank-error
done

echo "==> Flattening cost + accuracy passes into one row per grid point..." >&2
"$BINARY" flatten "$RAW_JSONL" --output "$GRID_JSONL"

echo "==> Reducing to atomic-cost table..." >&2
"$BINARY" atomic-costs "$GRID_JSONL" --output "$TABLE_JSON"

echo "" >&2
echo "Done. $TABLE_JSON" >&2
