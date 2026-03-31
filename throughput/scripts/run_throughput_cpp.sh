#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${THROUGHPUT_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs|hll|kll|octo|cms32k|cs32k) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, hll, kll, octo, cms32k, or cs32k" >&2
    exit 1
    ;;
esac

if [[ "${VARIANT}" == "octo" || "${VARIANT}" == "cs32k" ]]; then
  echo "${VARIANT} variant is Rust-only; skipping C++ build."
  exit 0
fi

VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_throughput"
DATA_DST="${THROUGHPUT_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
BUILD_DIR="${VARIANT_DIR}/cpp/build"
OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_results_cpp.csv}"

mkdir -p "${THROUGHPUT_DIR}/../input" "${VARIANT_DIR}/output" "${BUILD_DIR}"

rm -f "${BUILD_DIR}/CMakeCache.txt"
rm -rf "${BUILD_DIR}/CMakeFiles"

if [[ ! -f "${DATA_DST}" ]]; then
  "${THROUGHPUT_DIR}/../input/generate_zipf_data.sh"
fi

cmake -S "${VARIANT_DIR}/cpp" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release

case "${VARIANT}" in
  cms) "${BUILD_DIR}/cms_throughput" --data "${DATA_DST}" --output "${OUTPUT_PATH}" ;;
  cs) "${BUILD_DIR}/cs_throughput" --data "${DATA_DST}" --output "${OUTPUT_PATH}" ;;
  hll) "${BUILD_DIR}/hll_throughput" --data "${DATA_DST}" --output "${OUTPUT_PATH}" ;;
  kll) "${BUILD_DIR}/kll_throughput" --data "${DATA_DST}" --output "${OUTPUT_PATH}" ;;
  cms32k) "${BUILD_DIR}/cms32k_throughput" --data "${DATA_DST}" --output "${OUTPUT_PATH}" ;;
esac
