#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUTPUT_DIR="${ACCURACY_DIR}/output"
PLOTS_DIR="${ACCURACY_DIR}/plots"
CSV_PATH="${OUTPUT_DIR}/cms_accuracy_results.csv"
RUST_CSV="${OUTPUT_DIR}/rust_accuracy.csv"
CPP_CSV="${OUTPUT_DIR}/cpp_accuracy.csv"

mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

"${SCRIPT_DIR}/run_accuracy_rust.sh" "${RUST_CSV}"
"${SCRIPT_DIR}/run_accuracy_cpp.sh" "${CPP_CSV}"

{
  echo "implementation,language,seed,rows,cols,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error"
  tail -n +2 "${RUST_CSV}"
  cat "${CPP_CSV}"
} > "${CSV_PATH}"

python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
  --input "${CSV_PATH}" \
  --output "${PLOTS_DIR}/cms_accuracy_avg_relative_error.png" \
  --scatter-output "${PLOTS_DIR}/cms_accuracy_seed_scatter.png"

echo "Wrote ${CSV_PATH}"
echo "Wrote ${PLOTS_DIR}/cms_accuracy_avg_relative_error.png"
