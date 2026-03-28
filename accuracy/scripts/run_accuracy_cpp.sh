#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${ACCURACY_DIR}/.." && pwd)"
DATA_SRC="${REPO_ROOT}/input/benchmark_data_1m_int64.bin"
DATA_DST="${ACCURACY_DIR}/data/benchmark_data_1m_int64.bin"
BUILD_DIR="${ACCURACY_DIR}/cpp/build"
OUTPUT_PATH="${1:-/dev/stdout}"

mkdir -p "${ACCURACY_DIR}/data" "${BUILD_DIR}"
cp -f "${DATA_SRC}" "${DATA_DST}"

cmake -S "${ACCURACY_DIR}/cpp" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release
"${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" > "${OUTPUT_PATH}"
