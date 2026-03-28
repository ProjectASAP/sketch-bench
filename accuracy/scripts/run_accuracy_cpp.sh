#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DATA_DST="${ACCURACY_DIR}/data/benchmark_data_1m_int64_zipf_s11_k100000.bin"
BUILD_DIR="${ACCURACY_DIR}/cpp/build"
SUMMARY_OUTPUT_PATH="${1:-${ACCURACY_DIR}/output/cms_accuracy_results_cpp.csv}"
KEY_ERROR_OUTPUT_PATH="${2:-${ACCURACY_DIR}/output/cms_accuracy_key_median_errors_cpp.csv}"

mkdir -p "${ACCURACY_DIR}/data" "${ACCURACY_DIR}/output" "${BUILD_DIR}"

if [[ ! -f "${DATA_DST}" ]]; then
  "${ACCURACY_DIR}/data/generate_zipf_data.sh"
fi

cmake -S "${ACCURACY_DIR}/cpp" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release
"${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode summary > "${SUMMARY_OUTPUT_PATH}"
"${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode key-errors > "${KEY_ERROR_OUTPUT_PATH}"
