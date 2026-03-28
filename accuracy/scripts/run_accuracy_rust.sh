#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DATA_DST="${ACCURACY_DIR}/data/benchmark_data_1m_int64_zipf_s11_k100000.bin"
SUMMARY_OUTPUT_PATH="${1:-${ACCURACY_DIR}/output/cms_accuracy_results_rust.csv}"
KEY_ERROR_OUTPUT_PATH="${2:-${ACCURACY_DIR}/output/cms_accuracy_key_median_errors_rust.csv}"

mkdir -p "${ACCURACY_DIR}/data" "${ACCURACY_DIR}/output"

if [[ ! -f "${DATA_DST}" ]]; then
  "${ACCURACY_DIR}/data/generate_zipf_data.sh"
fi

if [[ ! -f "${ACCURACY_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${ACCURACY_DIR}/rust/Cargo.toml"
fi

cargo run --release --offline --manifest-path "${ACCURACY_DIR}/rust/Cargo.toml" -- \
  --data "${DATA_DST}" \
  --output-summary "${SUMMARY_OUTPUT_PATH}" \
  --output-key-errors "${KEY_ERROR_OUTPUT_PATH}"
