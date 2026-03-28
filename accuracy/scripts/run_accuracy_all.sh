#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUTPUT_DIR="${ACCURACY_DIR}/output"
PLOTS_DIR="${ACCURACY_DIR}/plots"
SUMMARY_CSV_PATH="${OUTPUT_DIR}/cms_accuracy_results.csv"
KEY_ERROR_CSV_PATH="${OUTPUT_DIR}/cms_accuracy_key_median_errors.csv"
RUST_SUMMARY_CSV="${OUTPUT_DIR}/cms_accuracy_results_rust.csv"
CPP_SUMMARY_CSV="${OUTPUT_DIR}/cms_accuracy_results_cpp.csv"
RUST_KEY_ERROR_CSV="${OUTPUT_DIR}/cms_accuracy_key_median_errors_rust.csv"
CPP_KEY_ERROR_CSV="${OUTPUT_DIR}/cms_accuracy_key_median_errors_cpp.csv"
SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error"
KEY_ERROR_HEADER="implementation,language,rows,cols,key,true_count,median_estimate,median_relative_error"

mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

"${SCRIPT_DIR}/run_accuracy_rust.sh" "${RUST_SUMMARY_CSV}" "${RUST_KEY_ERROR_CSV}"
"${SCRIPT_DIR}/run_accuracy_cpp.sh" "${CPP_SUMMARY_CSV}" "${CPP_KEY_ERROR_CSV}"

{
  echo "${SUMMARY_HEADER}"
  tail -n +2 "${RUST_SUMMARY_CSV}"
  cat "${CPP_SUMMARY_CSV}"
} > "${SUMMARY_CSV_PATH}"

{
  echo "${KEY_ERROR_HEADER}"
  tail -n +2 "${RUST_KEY_ERROR_CSV}"
  cat "${CPP_KEY_ERROR_CSV}"
} > "${KEY_ERROR_CSV_PATH}"

python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
  --input-key-errors "${KEY_ERROR_CSV_PATH}" \
  --output "${PLOTS_DIR}/cms_accuracy_avg_relative_error.png"

echo "Wrote ${SUMMARY_CSV_PATH}"
echo "Wrote ${KEY_ERROR_CSV_PATH}"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_from_8192.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_from_16384.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_2048.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_4096.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_8192.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_16384.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_32768.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_65536.png"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error_col_131072.png"
