#!/usr/bin/env bash
# Run throughput benchmarks through `sketchlib bench` and emit the
# legacy long-format CSVs under `output/throughput/`.
#
# Replaces the per-family binaries that used to live under
# `throughput/<family>/rust/` plus `throughput/polars_*/` and
# `throughput/octo/`. All of those are now reachable through the
# sketch-cli wrappers:
#
#   sketch-cli/src/wrappers/polars.rs    — *.polars   (exact baselines)
#   sketch-cli/src/wrappers/parallel.rs  — *.lib-fastpath-parallel (octo)
#   sketch-cli/src/wrappers/{cms,countsketch,hll,kll,dd,nitro}.rs  — regular impls
#
# Output layout mirrors the legacy `throughput/<family>/output/`
# shape so the plot scripts under `visualization/plots/throughput/`
# keep working without changes: each family writes
# `<family>_throughput_results_rust.csv` (insert) and, when
# accuracy ran, `<family>_throughput_query_results_rust.csv`.
# Octo-parallel rows land in `octo_throughput_results_rust.csv`.
#
# Usage:
#   scripts/run_throughput.sh                       # all families
#   scripts/run_throughput.sh --variant cms         # one family
#   scripts/run_throughput.sh --workers 1,2,4,8     # octo sweep
#   scripts/run_throughput.sh --output-dir /tmp/out
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

VARIANT="all"
OUTPUT_DIR="${REPO_ROOT}/output/throughput"
DATA="${THROUGHPUT_DATASET:-${REPO_ROOT}/input/benchmark_data_10m_int64_zipf_s11_k100000.bin}"
WORKERS_LIST="1,2,4,8"
RUNS="${THROUGHPUT_RUNS:-10}"
WARMUP="${THROUGHPUT_WARMUP:-3}"
ACCURACY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --variant)     VARIANT="$2"; shift 2 ;;
    --output-dir)  OUTPUT_DIR="$2"; shift 2 ;;
    --data)        DATA="$2"; shift 2 ;;
    --workers)     WORKERS_LIST="$2"; shift 2 ;;
    --accuracy)    ACCURACY="--accuracy"; shift ;;
    --runs)        RUNS="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,/^set/p' "$0" | sed -n 's/^# \{0,1\}//p'
      exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

mkdir -p "${OUTPUT_DIR}"
REPORT="${OUTPUT_DIR}/throughput.jsonl"
: > "${REPORT}"  # truncate

# Ensure the dataset exists. The legacy generator script is kept
# at its old location through the migration.
if [[ ! -f "${DATA}" ]]; then
  if [[ -x "${REPO_ROOT}/input/generate_zipf_data.sh" ]]; then
    "${REPO_ROOT}/input/generate_zipf_data.sh"
  else
    echo "dataset not found and no generator: ${DATA}" >&2
    exit 1
  fi
fi

run_family() {
  local FAMILY="$1"
  local IMPL_FILTER="${2:-all}"
  local CONFIG="${3:-}"
  local EXTRA="${4:-}"
  local -a CONFIG_ARGS=()
  if [[ -n "${CONFIG}" ]]; then
    CONFIG_ARGS=(--config "${CONFIG}")
  fi
  echo "===== throughput: ${FAMILY} (impl=${IMPL_FILTER}) ====="
  # shellcheck disable=SC2086
  cargo run --release --quiet -p sketch-cli -- bench \
    --sketch "${FAMILY}" --impl "${IMPL_FILTER}" \
    --input "${DATA}" --runs "${RUNS}" --warmup-runs "${WARMUP}" \
    ${ACCURACY} \
    "${CONFIG_ARGS[@]}" \
    --raw-csv "${OUTPUT_DIR}" \
    --report "${REPORT}" \
    ${EXTRA}
}

run_octo() {
  # Parallel-insert impl across the worker-count sweep. Each
  # invocation appends to `octo_throughput_results_rust.csv`.
  IFS=',' read -ra WORKERS <<<"${WORKERS_LIST}"
  for n in "${WORKERS[@]}"; do
    for fam_cfg in "cms|rows=5 cols=32768" "countsketch|rows=5 cols=32768" "hll|lg_k=14"; do
      local FAM="${fam_cfg%%|*}"
      local CFG="${fam_cfg#*|}"
      echo "===== throughput: ${FAM}/lib-fastpath-parallel workers=${n} ====="
      cargo run --release --quiet -p sketch-cli -- bench \
        --sketch "${FAM}" --impl lib-fastpath-parallel \
        --config "${CFG}" \
        --input "${DATA}" --runs "${RUNS}" --warmup-runs "${WARMUP}" \
        --workers "${n}" \
        --raw-csv "${OUTPUT_DIR}" \
        --report "${REPORT}"
    done
  done
}

case "${VARIANT}" in
  all)
    run_family hll
    run_family kll
    run_family cms
    run_family countsketch
    run_family dd
    run_family nitro
    run_family elastic
    run_family univmon
    run_octo
    ;;
  octo)         run_octo ;;
  cs)           run_family countsketch ;;
  *)            run_family "${VARIANT}" ;;
esac

echo "----"
echo "JSONL report : ${REPORT}"
echo "Legacy CSVs  : ${OUTPUT_DIR}/<family>_throughput_results_rust.csv"
echo "Octo CSV     : ${OUTPUT_DIR}/octo_throughput_results_rust.csv"
