#!/usr/bin/env bash
# Run throughput benchmarks through `approxbench bench` and emit the
# legacy long-format CSVs under `output/throughput/`.
#
# Replaces the per-algorithm binaries that used to live under
# `throughput/<algorithm>/rust/` plus `throughput/polars_*/` and
# `throughput/octo/`. All of those are now reachable through the
# aqpbm-cli wrappers:
#
#   sketch-bench/src/wrappers/polars.rs    — */polars   (exact baselines)
#   sketch-bench/src/wrappers/parallel.rs  — *-parallel/lib (octo)
#   sketch-bench/src/wrappers/{cms,countsketch,hll,kll,dd,nitro}.rs  — the rest
#
# `--variance` takes the structural variant too (`cms-fastpath-vector2d`, not
# `cms`), and `--library` takes the library alone. `--list-impls` prints the pairs.
#
# Output layout mirrors the legacy `throughput/<algorithm>/output/`
# shape so the plot scripts under `visualization/plots/throughput/`
# keep working without changes: each algorithm writes
# `<family>_throughput_results_rust.csv` (insert) and, when
# accuracy ran, `<family>_throughput_query_results_rust.csv`.
# One file per family: a structural variant is told apart by the
# `implementation` column inside it, not by a file of its own.
# Octo-parallel rows land in `octo_throughput_results_rust.csv`.
#
# Usage:
#   scripts/run_throughput.sh                       # all algorithms
#   scripts/run_throughput.sh --variant cms         # one algorithm
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

run_algorithm() {
  local ALGORITHM="$1"
  local IMPL_FILTER="${2:-all}"
  local CONFIG="${3:-}"
  local EXTRA="${4:-}"
  local -a CONFIG_ARGS=()
  if [[ -n "${CONFIG}" ]]; then
    CONFIG_ARGS=(--config "${CONFIG}")
  fi
  echo "===== throughput: ${ALGORITHM} (impl=${IMPL_FILTER}) ====="
  # shellcheck disable=SC2086
  cargo run --release --quiet -p aqpbm-cli -- sketchbench \
    --variance "${ALGORITHM}" --library "${IMPL_FILTER}" \
    --input "${DATA}" --runs "${RUNS}" --warmup-runs "${WARMUP}" \
    ${ACCURACY} \
    "${CONFIG_ARGS[@]}" \
    --raw-csv "${OUTPUT_DIR}" \
    --report "${REPORT}" \
    ${EXTRA}
}

run_octo() {
  # Parallel-insert rows across the worker-count sweep. Each
  # invocation appends to `octo_throughput_results_rust.csv`.
  #
  # Parallel insert is its own algorithm, and its per-worker sketch is a
  # compile-time shape, so the config below is the only one these rows build at.
  IFS=',' read -ra WORKERS <<<"${WORKERS_LIST}"
  for n in "${WORKERS[@]}"; do
    for algo_cfg in \
        "cms-fastpath-fixedmatrix-32k-parallel|rows=5 cols=32768" \
        "countsketch-fastpath-fixedmatrix-32k-parallel|rows=5 cols=32768" \
        "hll-fastpath-parallel|lg_k=14"; do
      local ALGO="${algo_cfg%%|*}"
      local CFG="${algo_cfg#*|}"
      echo "===== throughput: ${ALGO}/lib workers=${n} ====="
      cargo run --release --quiet -p aqpbm-cli -- sketchbench \
        --variance "${ALGO}" --library lib \
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
    run_algorithm hll
    run_algorithm kll
    run_algorithm cms
    run_algorithm countsketch
    run_octo
    ;;
  octo)         run_octo ;;
  cs)           run_algorithm countsketch ;;
  *)            run_algorithm "${VARIANT}" ;;
esac

echo "----"
echo "JSONL report : ${REPORT}"
echo "Legacy CSVs  : ${OUTPUT_DIR}/<family>_throughput_results_rust.csv"
echo "Octo CSV     : ${OUTPUT_DIR}/octo_throughput_results_rust.csv"
