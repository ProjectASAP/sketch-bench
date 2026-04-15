#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENERATOR_BIN="${SCRIPT_DIR}/generate_zipf_data"
NUM_VALUES="${1:-10000000}"
ZIPF_EXPONENT="${2:-1.1}"
SUPPORT_SIZE="${3:-100000}"
SEED="${4:-42}"
EXPONENT_TAG="$(printf '%s' "${ZIPF_EXPONENT}" | tr -d '.' | sed 's/0*$//')"
if [[ -z "${EXPONENT_TAG}" ]]; then
  EXPONENT_TAG="0"
fi
DATA_FILE="${SCRIPT_DIR}/benchmark_data_$((NUM_VALUES / 1000000))m_int64_zipf_s${EXPONENT_TAG}_k${SUPPORT_SIZE}.bin"

echo "========================================"
echo "Generating Zipf accuracy data"
echo "Target: ${DATA_FILE}"
echo "Parameters: draws=${NUM_VALUES} exponent=${ZIPF_EXPONENT} support=${SUPPORT_SIZE} seed=${SEED}"
echo "========================================"

rm -f "${GENERATOR_BIN}" "${DATA_FILE}"
g++ -std=c++17 -O3 "${SCRIPT_DIR}/generate_zipf_data.cpp" -o "${GENERATOR_BIN}"
(cd "${SCRIPT_DIR}" && ./generate_zipf_data "${NUM_VALUES}" "${ZIPF_EXPONENT}" "${SUPPORT_SIZE}" "${SEED}")

echo "Done."
