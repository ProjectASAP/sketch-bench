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
SUMMARY_OUTPUT_PATH="${1:-${VARIANT_DIR}/output/${RESULT_PREFIX}_results_rust.csv}"
KEY_SEED_ERROR_OUTPUT_PATH="${ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_seed_errors_rust.csv}"

mkdir -p "${ACCURACY_DIR}/../input" "${VARIANT_DIR}/output"

if [[ ! -f "${DATA_DST}" ]]; then
  "${ACCURACY_DIR}/../input/generate_zipf_data.sh"
fi

if [[ ! -f "${VARIANT_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml"
fi

if [[ "${VARIANT}" == "hll" || "${VARIANT}" == "kll" ]]; then
  cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}"
elif [[ "${VARIANT}" == "cms" ]]; then
  KEY_ERROR_OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_median_errors_rust.csv}"
  cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}" \
    --output-key-errors "${KEY_ERROR_OUTPUT_PATH}" \
    --output-key-seed-errors "${KEY_SEED_ERROR_OUTPUT_PATH}"
else
  KEY_ERROR_OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_key_median_errors_rust.csv}"
  cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}" \
    --output-key-errors "${KEY_ERROR_OUTPUT_PATH}"
fi
