#!/usr/bin/env bash
# run_profiles.sh — CPU-accuracy and memory-accuracy profiles
#
# Runs all four sketch families (hll, kll, cms, countsketch)
# across three workloads (zipf, uniform, file) using the default
# config grids. Emits one JSONL file per (family, workload) in
# output/profiles/, plus a combined all.jsonl.
#
# Usage:
#   scripts/run_profiles.sh             # build + run everything
#   scripts/run_profiles.sh --no-build  # skip cargo build
#   FAMILIES="hll kll" scripts/run_profiles.sh  # subset of families
#
# ── Key JSONL fields for analysis ──────────────────────────────
# Identity:
#   .sketch                            family name
#   .impl                              impl name
#   .sketch_config.params              config (lg_k / k / rows+cols / …)
#   .workload.shape                    "zipf" | "uniform" | "file"
#
# Memory (for memory-accuracy curve):
#   .bench.memory_bytes                sketch's own declared footprint (bytes)
#                                      ← most reliable; tied directly to params
#   .bench.heap_allocated_kb           jemalloc in-use at run-end (KB)
#                                      ← actual heap; includes allocator overhead
#   .bench.rss_peak_kb                 process RSS HWM (KB)
#                                      ← noisy across configs (process-wide HWM
#                                         only grows); use heap_allocated_kb instead
#
# CPU (for cpu-accuracy curve):
#   .bench.cpu_time_ms.user_ms.mean    CPU user time (ms, mean over runs)
#   .bench.cpu_time_ms.user_ms.ci95    [lo, hi] 95% CI
#   .bench.cpu_time_ms.sys_ms.mean     CPU sys time (ms)
#   .bench.wall_time_ms.mean           wall clock time (ms)
#
# Accuracy:
#   HLL:           .bench.accuracy.relative_error
#   KLL:           .bench.accuracy.mean_rank_err, .bench.accuracy.max_rank_err
#   CMS/CS (freq): .bench.accuracy.relative_error_mean
#                  .bench.accuracy.relative_error_p99
#                  .bench.accuracy.l1_err
#                  .bench.accuracy.min_true_count   (threshold applied)
#                  .bench.accuracy.filtered_out      (keys below threshold)
#
# Notes:
# - lib-fastpath-parallel impls have no accuracy comparator; they appear
#   in output with .bench.accuracy absent.
# - polars impls are DataFrame batch-insert baselines, not streaming
#   sketches; their memory/CPU cost is not directly comparable.
# - exact impls are zero-error baselines (unparameterized; run once).
# - For cms/countsketch under zipf, accuracy-min-count=10 restricts
#   relative-error to the heavy-hitter regime. Under uniform, 0 is used.
# ───────────────────────────────────────────────────────────────

set -euo pipefail

BUILD=1
if [[ "${1:-}" == "--no-build" ]]; then
    BUILD=0
fi

BINARY=./target/release/sketchlib
FAMILIES_DEFAULT="hll kll cms countsketch"
FAMILIES="${FAMILIES:-$FAMILIES_DEFAULT}"

# ── Tune these ──────────────────────────────────────────────────
RUNS=5
WARMUP=2
SIZE=1000000
CARDINALITY=100000
ZIPF_S=1.1
SEED=42
METRICS="cpu,memory,accuracy"
ACCURACY_PROBES=20000
# Zipf: restrict frequency rel-err to keys with count >= this value.
# Avoids count-1 rare-key noise dominating the mean rel-err for CMS/CS.
# Ignored by hll (cardinality) and kll (quantile) comparators.
ACCURACY_MIN_COUNT_ZIPF=10
# Uniform / file: all keys have similar counts; no filter.
ACCURACY_MIN_COUNT_OTHER=0
# ────────────────────────────────────────────────────────────────

OUT_DIR=output/profiles
mkdir -p "$OUT_DIR"

if [[ $BUILD -eq 1 ]]; then
    echo "==> Building sketchlib (release)..."
    cargo build -p sketch-cli --release
fi

if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: $BINARY not found. Run without --no-build." >&2
    exit 1
fi

# Truncate existing output files so reruns don't append duplicates
# (the CLI opens with O_APPEND; clean slate on each script run).
for FAMILY in $FAMILIES; do
    rm -f "$OUT_DIR/${FAMILY}_zipf.jsonl" \
          "$OUT_DIR/${FAMILY}_uniform.jsonl" \
          "$OUT_DIR/${FAMILY}_file.jsonl"
done
rm -f "$OUT_DIR/all.jsonl"

# ── Benchmark driver ─────────────────────────────────────────────
bench() {
    local family=$1; shift
    local label=$1; shift
    local min_count=$1; shift
    echo ""
    echo "─── $family / $label ───────────────────────────────────"
    "$BINARY" bench \
        --sketch    "$family" \
        --runs      "$RUNS" \
        --warmup-runs "$WARMUP" \
        --metrics   "$METRICS" \
        --accuracy \
        --accuracy-probes     "$ACCURACY_PROBES" \
        --accuracy-min-count  "$min_count" \
        --report    "$OUT_DIR/${family}_${label}.jsonl" \
        "$@"
}

for FAMILY in $FAMILIES; do
    # Zipf workload — heavy-hitter filter on for frequency sketches
    bench "$FAMILY" zipf "$ACCURACY_MIN_COUNT_ZIPF" \
        --workload zipf \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --zipf-s "$ZIPF_S" \
        --seed "$SEED"

    # Uniform workload — all keys have similar counts
    bench "$FAMILY" uniform "$ACCURACY_MIN_COUNT_OTHER" \
        --workload uniform \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --seed "$SEED"

    # Pre-built binary file — 1M int64, seed 42
    bench "$FAMILY" file "$ACCURACY_MIN_COUNT_OTHER" \
        --input input/benchmark_data_1m_int64.bin
done

# ── Combine ──────────────────────────────────────────────────────
cat "$OUT_DIR"/*_zipf.jsonl \
    "$OUT_DIR"/*_uniform.jsonl \
    "$OUT_DIR"/*_file.jsonl \
    2>/dev/null \
    > "$OUT_DIR/all.jsonl"

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "Done. Output in $OUT_DIR/"
echo ""
echo "  Per-run files:"
for F in "$OUT_DIR"/*.jsonl; do
    [[ "$F" == *all.jsonl ]] && continue
    printf "    %-40s  %d records\n" "$(basename "$F")" "$(wc -l < "$F")"
done
echo ""
printf "  %-40s  %d records\n" "all.jsonl (combined)" "$(wc -l < "$OUT_DIR/all.jsonl")"
echo ""
echo "  Quick peek (first hll/zipf record):"
head -1 "$OUT_DIR/hll_zipf.jsonl" 2>/dev/null \
    | python3 -m json.tool 2>/dev/null | head -40 || true
