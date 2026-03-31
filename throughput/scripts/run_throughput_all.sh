#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${THROUGHPUT_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs|hll|kll) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, hll, or kll" >&2
    exit 1
    ;;
esac

VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_throughput"
OUTPUT_DIR="${VARIANT_DIR}/output"
PLOTS_DIR="${THROUGHPUT_DIR}/plots/${VARIANT}"
SUMMARY_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_results.csv"
RUST_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv"
CPP_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv"
case "${VARIANT}" in
  kll)
    SUMMARY_HEADER="implementation,language,run,k,total_items,total_nanoseconds,throughput_items_per_sec"
    ;;
  hll)
    SUMMARY_HEADER="implementation,language,run,lg_k,registers,total_items,total_nanoseconds,throughput_items_per_sec"
    ;;
  *)
    SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec"
    ;;
esac

mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

"${SCRIPT_DIR}/run_throughput_rust.sh" "${VARIANT}" "${RUST_SUMMARY_CSV}"
"${SCRIPT_DIR}/run_throughput_cpp.sh" "${VARIANT}" "${CPP_SUMMARY_CSV}"

{
  echo "${SUMMARY_HEADER}"
  tail -n +2 "${RUST_SUMMARY_CSV}"
  tail -n +2 "${CPP_SUMMARY_CSV}"
} > "${SUMMARY_CSV_PATH}"

case "${VARIANT}" in
  cms)
    python3 "${SCRIPT_DIR}/plot_cms_throughput.py" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
    ;;
  cs)
    python3 "${SCRIPT_DIR}/plot_cs_throughput.py" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
    ;;
  hll)
    python3 "${SCRIPT_DIR}/plot_hll_throughput.py" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
    ;;
  kll)
    python3 "${SCRIPT_DIR}/plot_kll_throughput.py" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
    ;;
esac

echo "Wrote ${SUMMARY_CSV_PATH}"
echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
