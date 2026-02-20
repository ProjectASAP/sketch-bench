#!/bin/bash
# Utility script to regenerate the shared benchmark data file.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_FILE="$SCRIPT_DIR/benchmark_data_1m_int64.bin"

echo "========================================"
echo "Generating benchmark data"
echo "Target: $DATA_FILE"
echo "========================================"

rm -f "$DATA_FILE"
g++ -std=c++17 -O3 "$SCRIPT_DIR/generate_data.cpp" -o "$SCRIPT_DIR/generate_data"
(cd "$SCRIPT_DIR" && ./generate_data)

echo "Done."
