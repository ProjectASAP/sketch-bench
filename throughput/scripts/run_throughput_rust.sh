#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${THROUGHPUT_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms or cs" >&2
    exit 1
    ;;
esac

VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_throughput"
DATA_DST="${THROUGHPUT_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
OUTPUT_PATH="${2:-${VARIANT_DIR}/output/${RESULT_PREFIX}_results_rust.csv}"

mkdir -p "${THROUGHPUT_DIR}/../input" "${VARIANT_DIR}/output"

if [[ ! -f "${DATA_DST}" ]]; then
  "${THROUGHPUT_DIR}/../input/generate_zipf_data.sh"
fi

if [[ ! -f "${VARIANT_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml"
fi

cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
  --data "${DATA_DST}" \
  --output "${OUTPUT_PATH}"
