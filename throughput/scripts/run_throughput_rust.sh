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

# octo has no query binary
if [[ "${VARIANT}" == "octo" && "${OP}" == "query" ]]; then
  echo "octo has no query binary; skipping"
  exit 0
fi

VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
DATA_DST="${THROUGHPUT_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"

if [[ "${OP}" == "insert" ]]; then
  BIN_NAME="${VARIANT}_throughput"
  DEFAULT_OUTPUT="${VARIANT_DIR}/output/${VARIANT}_throughput_results_rust.csv"
else
  BIN_NAME="${VARIANT}_throughput_query"
  DEFAULT_OUTPUT="${VARIANT_DIR}/output/${VARIANT}_throughput_query_results_rust.csv"
fi

OUTPUT_PATH="${OUTPUT_ARG:-${DEFAULT_OUTPUT}}"

mkdir -p "${THROUGHPUT_DIR}/../input" "${VARIANT_DIR}/output"

if [[ ! -f "${DATA_DST}" ]]; then
  "${THROUGHPUT_DIR}/../input/generate_zipf_data.sh"
fi

if [[ ! -f "${VARIANT_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml"
fi

cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" --bin "${BIN_NAME}" -- \
  --data "${DATA_DST}" \
  --output "${OUTPUT_PATH}"
