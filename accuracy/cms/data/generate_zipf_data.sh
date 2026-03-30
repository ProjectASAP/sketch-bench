#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENERATOR_BIN="${SCRIPT_DIR}/generate_zipf_data"
DATA_FILE="${SCRIPT_DIR}/benchmark_data_10m_int64_zipf_s11_k100000.bin"

echo "========================================"
echo "Generating Zipf accuracy data"
echo "Target: ${DATA_FILE}"
echo "Parameters: draws=10000000 exponent=1.1 support=100000 seed=42"
echo "========================================"

rm -f "${GENERATOR_BIN}" "${DATA_FILE}"
g++ -std=c++17 -O3 "${SCRIPT_DIR}/generate_zipf_data.cpp" -o "${GENERATOR_BIN}"
(cd "${SCRIPT_DIR}" && ./generate_zipf_data)

echo "Done."
