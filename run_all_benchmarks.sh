#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "========================================"
echo "Running rust bench_sketches benchmarks"
echo "========================================"
(cd "$ROOT_DIR/rust" && bash "run_benchmark.sh")

echo "========================================"
echo "Running cpp benchmarks"
echo "========================================"
(cd "$ROOT_DIR/cpp" && bash "run_benchmark.sh")

echo "========================================"
echo "All benchmark suites completed"
echo "Aggregated reports updated in each benchmark directory"
echo "========================================"

echo ""
echo "========================================"
echo "All tasks completed"
echo "========================================"
