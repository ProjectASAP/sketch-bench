#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${3:-${ACCURACY_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs|hll|kll) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, hll, or kll" >&2
    exit 1
    ;;
esac

VARIANT_DIR="${ACCURACY_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_accuracy"
DATA_DST="${ACCURACY_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
BUILD_DIR="${VARIANT_DIR}/cpp/build"
SUMMARY_OUTPUT_PATH="${1:-${VARIANT_DIR}/output/${RESULT_PREFIX}_results_cpp.csv}"
KEY_SEED_ERROR_OUTPUT_PATH="${ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_seed_errors_cpp.csv}"

mkdir -p "${ACCURACY_DIR}/../input" "${VARIANT_DIR}/output" "${BUILD_DIR}"

# The accuracy tree was split into per-variant source roots, so an older cache
# may still point at the previous top-level source directory.
rm -f "${BUILD_DIR}/CMakeCache.txt"
rm -rf "${BUILD_DIR}/CMakeFiles"

if [[ ! -f "${DATA_DST}" ]]; then
  "${ACCURACY_DIR}/../input/generate_zipf_data.sh"
fi

cmake -S "${VARIANT_DIR}/cpp" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release

if [[ "${VARIANT}" == "hll" ]]; then
  "${BUILD_DIR}/hll_accuracy" --data "${DATA_DST}" > "${SUMMARY_OUTPUT_PATH}"
elif [[ "${VARIANT}" == "kll" ]]; then
  "${BUILD_DIR}/kll_accuracy" --data "${DATA_DST}" > "${SUMMARY_OUTPUT_PATH}"
else
  KEY_ERROR_OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_median_errors_cpp.csv}"
  "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode summary > "${SUMMARY_OUTPUT_PATH}"
  "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode key-errors > "${KEY_ERROR_OUTPUT_PATH}"
  if [[ "${VARIANT}" == "cms" ]]; then
    "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode key-seed-errors > "${KEY_SEED_ERROR_OUTPUT_PATH}"
  fi
fi
