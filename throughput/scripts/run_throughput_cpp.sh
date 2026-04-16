#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${THROUGHPUT_VARIANT:-cms}}"
OUTPUT_ARG="${2:-}"
OP="${3:-${THROUGHPUT_OP:-insert}}"

case "${VARIANT}" in
  cms|cs|hll|kll|octo|cms32k|cs32k) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, hll, kll, octo, cms32k, or cs32k" >&2
    exit 1
    ;;
esac

case "${OP}" in
  insert|query) ;;
  *)
    echo "unsupported op: ${OP}; expected insert or query" >&2
    exit 1
    ;;
esac

# Rust-only variants
if [[ "${VARIANT}" == "octo" || "${VARIANT}" == "cs32k" ]]; then
  echo "${VARIANT} variant is Rust-only; skipping C++ build."
  exit 0
fi

# CS has no C++ query binary (AWS insert-optimized has no get_estimate)
if [[ "${VARIANT}" == "cs" && "${OP}" == "query" ]]; then
  echo "cs has no C++ query binary; skipping"
  exit 0
fi

VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
DEFAULT_DATASET="${THROUGHPUT_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
DATA_DST="${THROUGHPUT_DATASET:-${ACCURACY_DATASET:-${DEFAULT_DATASET}}}"
BUILD_DIR="${VARIANT_DIR}/cpp/build"

if [[ "${OP}" == "insert" ]]; then
  BIN_NAME="${VARIANT}_throughput"
  DEFAULT_OUTPUT="${VARIANT_DIR}/output/${VARIANT}_throughput_results_cpp.csv"
else
  BIN_NAME="${VARIANT}_throughput_query"
  DEFAULT_OUTPUT="${VARIANT_DIR}/output/${VARIANT}_throughput_query_results_cpp.csv"
fi

OUTPUT_PATH="${OUTPUT_ARG:-${DEFAULT_OUTPUT}}"

mkdir -p "${THROUGHPUT_DIR}/../input" "${VARIANT_DIR}/output" "${BUILD_DIR}"

rm -f "${BUILD_DIR}/CMakeCache.txt"
rm -rf "${BUILD_DIR}/CMakeFiles"

if [[ "${DATA_DST}" == "${DEFAULT_DATASET}" && ! -f "${DATA_DST}" ]]; then
  "${THROUGHPUT_DIR}/../input/generate_zipf_data.sh"
elif [[ ! -f "${DATA_DST}" ]]; then
  echo "dataset not found: ${DATA_DST}" >&2
  exit 1
fi

cmake -S "${VARIANT_DIR}/cpp" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release --target "${BIN_NAME}"

"${BUILD_DIR}/${BIN_NAME}" --data "${DATA_DST}" --output "${OUTPUT_PATH}"
