#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

run_op() {
  local VARIANT="$1"
  local OP="$2"
  echo "===== Throughput: ${VARIANT} (${OP}) ====="

  local VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
  local OUTPUT_DIR="${VARIANT_DIR}/output"
  local PLOTS_DIR="${THROUGHPUT_DIR}/plots/${VARIANT}"
  mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

  local RESULT_PREFIX SUMMARY_HEADER
  if [[ "${OP}" == "insert" ]]; then
    RESULT_PREFIX="${VARIANT}_throughput"
    case "${VARIANT}" in
      kll) SUMMARY_HEADER="implementation,language,run,k,total_items,total_nanoseconds,throughput_items_per_sec" ;;
      hll) SUMMARY_HEADER="implementation,language,run,lg_k,registers,total_items,total_nanoseconds,throughput_items_per_sec" ;;
      *)   SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec" ;;
    esac
  else
    RESULT_PREFIX="${VARIANT}_throughput_query"
    case "${VARIANT}" in
      kll) SUMMARY_HEADER="implementation,language,run,k,total_items,repeat,percentile,call_index,nanoseconds,estimate" ;;
      hll) SUMMARY_HEADER="implementation,language,run,lg_k,registers,total_items,call_index,nanoseconds,estimate" ;;
      *)   SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,total_queries,total_nanoseconds,throughput_queries_per_sec" ;;
    esac
  fi

  local SUMMARY_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_results.csv"
  local RUST_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv"
  local CPP_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv"

  local RUST_ONLY=false
  case "${VARIANT}" in
    octo|cs32k) RUST_ONLY=true ;;
  esac
  # For query op, cs also lacks a C++ binary
  if [[ "${OP}" == "query" && "${VARIANT}" == "cs" ]]; then
    RUST_ONLY=true
  fi
  # Octo has no query binary at all
  if [[ "${OP}" == "query" && "${VARIANT}" == "octo" ]]; then
    echo "octo has no query benchmark; skipping"
    return 0
  fi

  if [[ "${VARIANT}" == "octo" && "${OP}" == "insert" ]]; then
    "${SCRIPT_DIR}/run_throughput_rust.sh" "${VARIANT}" "${RUST_SUMMARY_CSV}" "${OP}"
    python3 "${SCRIPT_DIR}/plot_octo_throughput.py" \
      --input "${RUST_SUMMARY_CSV}" \
      --output-cms "${PLOTS_DIR}/${RESULT_PREFIX}_cms.png" \
      --output-cs "${PLOTS_DIR}/${RESULT_PREFIX}_cs.png" \
      --output-hll "${PLOTS_DIR}/${RESULT_PREFIX}_hll.png"
    echo "Wrote ${RUST_SUMMARY_CSV}"
    return 0
  fi

  "${SCRIPT_DIR}/run_throughput_rust.sh" "${VARIANT}" "${RUST_SUMMARY_CSV}" "${OP}"

  if [[ "${RUST_ONLY}" == "true" ]]; then
    cp "${RUST_SUMMARY_CSV}" "${SUMMARY_CSV_PATH}"
  else
    "${SCRIPT_DIR}/run_throughput_cpp.sh" "${VARIANT}" "${CPP_SUMMARY_CSV}" "${OP}"
    {
      echo "${SUMMARY_HEADER}"
      tail -n +2 "${RUST_SUMMARY_CSV}"
      tail -n +2 "${CPP_SUMMARY_CSV}"
    } > "${SUMMARY_CSV_PATH}"
  fi

  local PLOT_SCRIPT=""
  local PLOT_OUTPUT=""
  if [[ "${OP}" == "insert" ]]; then
    case "${VARIANT}" in
      cms)    PLOT_SCRIPT="plot_cms_throughput.py" ;;
      cs)     PLOT_SCRIPT="plot_cs_throughput.py" ;;
      hll)    PLOT_SCRIPT="plot_hll_throughput.py" ;;
      kll)    PLOT_SCRIPT="plot_kll_throughput.py" ;;
      cms32k) PLOT_SCRIPT="plot_cms32k_throughput.py" ;;
      cs32k)  PLOT_SCRIPT="plot_cs32k_throughput.py" ;;
    esac
    PLOT_OUTPUT="${PLOTS_DIR}/${VARIANT}_throughput_insertion.png"
  else
    case "${VARIANT}" in
      cms)    PLOT_SCRIPT="plot_cms_throughput_query.py" ;;
      cs)     PLOT_SCRIPT="plot_cs_throughput_query.py" ;;
      hll)    PLOT_SCRIPT="plot_hll_throughput_query.py" ;;
      kll)    PLOT_SCRIPT="plot_kll_throughput_query.py" ;;
      cms32k) PLOT_SCRIPT="plot_cms32k_throughput_query.py" ;;
      cs32k)  PLOT_SCRIPT="plot_cs32k_throughput_query.py" ;;
    esac
    PLOT_OUTPUT="${PLOTS_DIR}/${VARIANT}_throughput_query.png"
  fi

  if [[ -n "${PLOT_SCRIPT}" && -f "${SCRIPT_DIR}/${PLOT_SCRIPT}" ]]; then
    python3 "${SCRIPT_DIR}/${PLOT_SCRIPT}" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOT_OUTPUT}"
    echo "Wrote ${PLOT_OUTPUT}"
  fi

  echo "Wrote ${SUMMARY_CSV_PATH}"
}

# Parse args: [VARIANT] [--op insert|query|both]
REQUESTED="all"
OP_REQUESTED="both"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --op)
      OP_REQUESTED="${2:-both}"; shift 2 ;;
    --op=*)
      OP_REQUESTED="${1#--op=}"; shift ;;
    -h|--help)
      echo "Usage: $0 [VARIANT] [--op insert|query|both]"
      echo "VARIANT: cms|cs|hll|kll|octo|cms32k|cs32k|all (default: all)"
      exit 0 ;;
    *)
      REQUESTED="$1"; shift ;;
  esac
done

REQUESTED="${REQUESTED:-${THROUGHPUT_VARIANT:-all}}"

case "${OP_REQUESTED}" in
  insert|query|both) ;;
  *)
    echo "unsupported --op: ${OP_REQUESTED}; expected insert, query, or both" >&2
    exit 1 ;;
esac

case "${REQUESTED}" in
  cms|cs|hll|kll|octo|cms32k|cs32k) VARIANTS=("${REQUESTED}") ;;
  all) VARIANTS=(cms cs hll kll octo cms32k cs32k) ;;
  *)
    echo "unsupported variant: ${REQUESTED}; expected cms, cs, hll, kll, octo, cms32k, cs32k, or all" >&2
    exit 1 ;;
esac

if [[ "${OP_REQUESTED}" == "both" ]]; then
  OPS=(insert query)
else
  OPS=("${OP_REQUESTED}")
fi

for v in "${VARIANTS[@]}"; do
  for op in "${OPS[@]}"; do
    run_op "$v" "$op"
  done
done
