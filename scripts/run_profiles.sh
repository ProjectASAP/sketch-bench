#!/usr/bin/env bash
# run_profiles.sh — CPU-accuracy and memory-accuracy profiles
#
# Runs the four sketch families (hll, kll, cms, countsketch)
# across three workloads (zipf, uniform, file) using the default
# config grids. Emits one JSONL file per (algorithm, workload) in
# output/profiles/, plus a combined all.jsonl.
#
# Usage:
#   scripts/run_profiles.sh             # build + run everything
#   scripts/run_profiles.sh --no-build  # skip cargo build
#   ALGORITHMS="hll kll" scripts/run_profiles.sh  # subset of algorithms
#
# ── Key JSONL fields for analysis ──────────────────────────────
# Identity:
#   .sketch                            algorithm name
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
#   .bench.cpu_time_ms.user_ms.ci95    [lo, hi] 95% CI — only under
#                                      `--repeats N`, N >= 2. This script
#                                      does not pass it, so expect it absent.
#   .bench.cpu_time_ms.sys_ms.mean     CPU sys time (ms)
#   .bench.wall_time_ms.mean           wall clock time (ms)
#
# Accuracy:
#   HLL:           .bench.accuracy.relative_error
#   KLL:           .bench.accuracy.mean_rank_err, .bench.accuracy.max_rank_err
#   CMS/CS (freq): .bench.accuracy.are_top10 / are_top100  (per-top-k error;
#                  are_all == the legacy relative_error_mean, dominated by
#                  count-1 keys — compare against the `null` impl, which is 1.0)
#                  .bench.accuracy.relative_error_p99
#                  .bench.accuracy.l1_err
#
# Notes:
# - the `*-parallel` algorithms have no accuracy comparator; they appear
#   in output with .bench.accuracy absent.
# - polars impls are DataFrame batch-insert baselines, not streaming
#   sketches; their memory/CPU cost is not directly comparable.
# - exact impls are zero-error baselines (unparameterized; run once).
# - The heavy-hitter regime for cms/countsketch under zipf is read off
#   the are_top1/10/100/1000 prefixes, not selected up front: the
#   `--accuracy-min-count` threshold this script used to pass was
#   removed because its empty-result fallback silently substituted a
#   different population under the same metric name.
# ───────────────────────────────────────────────────────────────

set -euo pipefail

BUILD=1
if [[ "${1:-}" == "--no-build" ]]; then
    BUILD=0
fi

BINARY=./target/release/approxbench
ALGORITHMS_DEFAULT="hll kll cms countsketch"
ALGORITHMS="${ALGORITHMS:-$ALGORITHMS_DEFAULT}"

# ── Tune these ──────────────────────────────────────────────────
RUNS=5
WARMUP=2
SIZE=1000000
CARDINALITY=100000
ZIPF_S=1.1
SEED=42
METRICS="cpu,memory,accuracy,throughput"
ACCURACY_PROBES=20000
# ────────────────────────────────────────────────────────────────

OUT_DIR=output/profiles
mkdir -p "$OUT_DIR"

if [[ $BUILD -eq 1 ]]; then
    echo "==> Building approxbench (release)..."
    cargo build -p aqpbm-cli --release
fi

if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: $BINARY not found. Run without --no-build." >&2
    exit 1
fi

# Truncate existing output files so reruns don't append duplicates
# (the CLI opens with O_APPEND; clean slate on each script run).
for ALGORITHM in $ALGORITHMS; do
    rm -f "$OUT_DIR/${ALGORITHM}_zipf.jsonl" \
          "$OUT_DIR/${ALGORITHM}_uniform.jsonl" \
          "$OUT_DIR/${ALGORITHM}_file.jsonl"
done
rm -f "$OUT_DIR/all.jsonl"

# ── Benchmark driver ─────────────────────────────────────────────
bench() {
    local algorithm=$1; shift
    local label=$1; shift
    echo ""
    echo "─── $algorithm / $label ───────────────────────────────────"
    "$BINARY" sketchbench \
        --algorithm  "$algorithm" \
        --runs      "$RUNS" \
        --warmup-runs "$WARMUP" \
        --metrics   "$METRICS" \
        --accuracy \
        --accuracy-probes     "$ACCURACY_PROBES" \
        --report    "$OUT_DIR/${algorithm}_${label}.jsonl" \
        "$@"
}

for ALGORITHM in $ALGORITHMS; do
    # Zipf workload — skewed, so the are_topN prefixes separate
    bench "$ALGORITHM" zipf \
        --workload zipf \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --zipf-s "$ZIPF_S" \
        --seed "$SEED"

    # Uniform workload — all keys have similar counts
    bench "$ALGORITHM" uniform \
        --workload uniform \
        --size "$SIZE" \
        --cardinality "$CARDINALITY" \
        --seed "$SEED"

    # Pre-built binary file — 1M int64, seed 42
    bench "$ALGORITHM" file \
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
