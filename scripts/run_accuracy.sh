#!/usr/bin/env bash
# Run accuracy benchmarks through `approxbench bench --accuracy` and
# emit the legacy long-format CSVs under `output/accuracy/`.
#
# Replaces `accuracy/<statistic>/rust/` per-statistic binaries.
# Each algorithm's comparator runs inside the aqpbm-cli runner; the
# per-call query CSV (hll/kll/dd) is enabled because `--accuracy`
# + `--raw-csv` are paired here.
#
# Usage:
#   scripts/run_accuracy.sh                  # all statistics
#   scripts/run_accuracy.sh --variant hll
#   scripts/run_accuracy.sh --probes 50000
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

VARIANT="all"
OUTPUT_DIR="${REPO_ROOT}/output/accuracy"
DATA="${ACCURACY_DATASET:-${REPO_ROOT}/input/benchmark_data_10m_int64_zipf_s11_k100000.bin}"
RUNS="${ACCURACY_RUNS:-10}"
WARMUP="${ACCURACY_WARMUP:-3}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --variant)         VARIANT="$2"; shift 2 ;;
    --output-dir)      OUTPUT_DIR="$2"; shift 2 ;;
    --data)            DATA="$2"; shift 2 ;;
    # Removed with `--accuracy-min-count`: it asked for a heavy-hitter
    # threshold chosen in advance, and its empty-result fallback
    # substituted a different population under the same metric name.
    # The `are_top1/10/100/1000` prefixes answer the same question
    # without one. Fail loudly rather than ignore a filter someone
    # believes is being applied.
    --min-true-count)
      echo "--min-true-count was removed; read .bench.accuracy.are_top10 / are_top100 instead" >&2
      exit 1 ;;
    --runs)            RUNS="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,/^set/p' "$0" | sed -n 's/^# \{0,1\}//p'
      exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

mkdir -p "${OUTPUT_DIR}"
REPORT="${OUTPUT_DIR}/accuracy.jsonl"
: > "${REPORT}"

if [[ ! -f "${DATA}" ]]; then
  if [[ -x "${REPO_ROOT}/input/generate_zipf_data.sh" ]]; then
    "${REPO_ROOT}/input/generate_zipf_data.sh"
  else
    echo "dataset not found: ${DATA}" >&2
    exit 1
  fi
fi

run_algorithm() {
  local ALGORITHM="$1"
  echo "===== accuracy: ${ALGORITHM} ====="
  cargo run --release --quiet -p aqpbm-cli -- sketchbench \
    --variant "${ALGORITHM}" --library all \
    --input "${DATA}" --runs "${RUNS}" --warmup-runs "${WARMUP}" \
    --accuracy \
    --raw-csv "${OUTPUT_DIR}" \
    --report "${REPORT}"
}

case "${VARIANT}" in
  all)
    # Statistic groupings (legacy accuracy/<statistic>/):
    #   cardinality  → hll
    #   frequency    → cms + countsketch + elastic
    #   quantile     → kll + dd
    run_algorithm hll
    run_algorithm kll
    run_algorithm cms
    run_algorithm countsketch
    run_algorithm dd
    run_algorithm elastic
    ;;
  cardinality)  run_algorithm hll ;;
  frequency)    run_algorithm cms; run_algorithm countsketch; run_algorithm elastic ;;
  quantile)     run_algorithm kll; run_algorithm dd ;;
  cs)           run_algorithm countsketch ;;
  *)            run_algorithm "${VARIANT}" ;;
esac

echo "----"
echo "JSONL report : ${REPORT}"
echo "Legacy CSVs  : ${OUTPUT_DIR}/<family>_throughput_*_results_rust.csv"
