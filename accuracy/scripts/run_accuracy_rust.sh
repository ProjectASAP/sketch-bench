#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${ACCURACY_DIR}/.." && pwd)"
DATA_SRC="${REPO_ROOT}/input/benchmark_data_1m_int64.bin"
DATA_DST="${ACCURACY_DIR}/data/benchmark_data_1m_int64.bin"
OUTPUT_PATH="${1:-${ACCURACY_DIR}/output/cms_accuracy_results.csv}"

mkdir -p "${ACCURACY_DIR}/data" "${ACCURACY_DIR}/output"
cp -f "${DATA_SRC}" "${DATA_DST}"

if [[ ! -f "${ACCURACY_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${ACCURACY_DIR}/rust/Cargo.toml"
fi

cargo run --release --offline --manifest-path "${ACCURACY_DIR}/rust/Cargo.toml" -- \
  --data "${DATA_DST}" \
  --output "${OUTPUT_PATH}"
