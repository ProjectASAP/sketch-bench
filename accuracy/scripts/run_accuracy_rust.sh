#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${3:-${ACCURACY_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms or cs" >&2
    exit 1
    ;;
esac

VARIANT_DIR="${ACCURACY_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_accuracy"
DATA_DST="${ACCURACY_DIR}/data/benchmark_data_1m_int64_zipf_s11_k100000.bin"
SUMMARY_OUTPUT_PATH="${1:-${VARIANT_DIR}/output/${RESULT_PREFIX}_results_rust.csv}"
KEY_ERROR_OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_median_errors_rust.csv}"

mkdir -p "${ACCURACY_DIR}/data" "${VARIANT_DIR}/output"

if [[ ! -f "${DATA_DST}" ]]; then
  "${ACCURACY_DIR}/data/generate_zipf_data.sh"
fi

if [[ ! -f "${VARIANT_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml"
fi

cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
  --data "${DATA_DST}" \
  --output-summary "${SUMMARY_OUTPUT_PATH}" \
  --output-key-errors "${KEY_ERROR_OUTPUT_PATH}"
