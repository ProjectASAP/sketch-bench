#!/usr/bin/env bash
# Run accuracy benchmarks through `sketchlib bench --accuracy` and
# emit the legacy long-format CSVs under `output/accuracy/`.
#
# Replaces `accuracy/<statistic>/rust/` per-statistic binaries.
# Each family's comparator runs inside the sketch-cli runner; the
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
PROBES="${ACCURACY_PROBES:-100000}"
RUNS="${ACCURACY_RUNS:-10}"
WARMUP="${ACCURACY_WARMUP:-3}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --variant)         VARIANT="$2"; shift 2 ;;
    --output-dir)      OUTPUT_DIR="$2"; shift 2 ;;
    --data)            DATA="$2"; shift 2 ;;
    --probes)          PROBES="$2"; shift 2 ;;
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

run_family() {
  local FAMILY="$1"
  echo "===== accuracy: ${FAMILY} ====="
  cargo run --release --quiet -p sketch-cli -- bench \
    --sketch "${FAMILY}" --impl all \
    --input "${DATA}" --runs "${RUNS}" --warmup-runs "${WARMUP}" \
    --accuracy --accuracy-probes "${PROBES}" \
    --raw-csv "${OUTPUT_DIR}" \
    --report "${REPORT}"
}

case "${VARIANT}" in
  all)
    # Statistic groupings (legacy accuracy/<statistic>/):
    #   cardinality  → hll
    #   frequency    → cms + countsketch + elastic
    #   quantile     → kll + dd
    run_family hll
    run_family kll
    run_family cms
    run_family countsketch
    run_family dd
    run_family elastic
    ;;
  cardinality)  run_family hll ;;
  frequency)    run_family cms; run_family countsketch; run_family elastic ;;
  quantile)     run_family kll; run_family dd ;;
  cs)           run_family countsketch ;;
  *)            run_family "${VARIANT}" ;;
esac

echo "----"
echo "JSONL report : ${REPORT}"
echo "Legacy CSVs  : ${OUTPUT_DIR}/<family>_throughput_*_results_rust.csv"
