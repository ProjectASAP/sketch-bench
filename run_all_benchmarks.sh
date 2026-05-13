#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "========================================"
echo "Running cpp benchmarks"
echo "========================================"
(cd "$ROOT_DIR/cpp" && bash "run_benchmark.sh")

# Per-sketch Rust throughput harnesses live under throughput/<family>/rust/
# and are orchestrated by scripts/run_all.py. Run those separately.

echo "========================================"
echo "All benchmark suites completed"
echo "Aggregated reports updated in each benchmark directory"
echo "========================================"

echo ""
echo "========================================"
echo "All tasks completed"
echo "========================================"
