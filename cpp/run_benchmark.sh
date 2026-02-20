#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"

PIN_CORE="${PIN_CORE:-0}"
TRIALS="${TRIALS:-10}"
GOOGLE_BENCHMARK_ROOT="${GOOGLE_BENCHMARK_ROOT:-$HOME/benchmark}"

BINARIES=(
  cs_naive
  cs_fastrange
  cs_fixed_size
  cs_final
  cs_final_no_murmur_unroll
  cs_datasketches
  kll_naive
  kll_datasketches
  kll_no_self_move_protection
  kll_pcg_random
  kll_no_min_max
  kll_cached_level_capacities
  kll_final
)

taskset_prefix=()
if command -v taskset >/dev/null 2>&1; then
  taskset_prefix=(taskset -c "${PIN_CORE}")
  echo "CPU pinning enabled via taskset on core ${PIN_CORE}"
else
  echo "taskset not available; CPU pinning skipped."
fi

mkdir -p "${BUILD_DIR}"

echo "Building C++ benchmarks..."
cmake -S "${SCRIPT_DIR}" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release \
  -DGOOGLE_BENCHMARK_ROOT="${GOOGLE_BENCHMARK_ROOT}"
cmake --build "${BUILD_DIR}" --config Release
echo "Build complete."
echo ""

export SKETCH_BENCH_ROOT="${ROOT_DIR}"

echo "Running C++ benchmarks..."
for bin in "${BINARIES[@]}"; do
  echo "----------------------------------------"
  echo "Benchmark: ${bin}"
  echo "----------------------------------------"
  if [[ ! -x "${BUILD_DIR}/${bin}" ]]; then
    echo "Skipping ${bin}: binary not found at ${BUILD_DIR}/${bin}"
    continue
  fi

  output_file="${SCRIPT_DIR}/output/${bin}.jsonl"
  tmp_json="$(mktemp)"
  mkdir -p "$(dirname "${output_file}")"
  echo "Running Google Benchmark with ${TRIALS} repetitions..."
  "${taskset_prefix[@]}" "${BUILD_DIR}/${bin}" \
    --benchmark_out_format=json \
    --benchmark_out="${tmp_json}" \
    --benchmark_repetitions="${TRIALS}" \
    --benchmark_report_aggregates_only=false

  python3 - "${tmp_json}" "cpp_${bin}" > "${output_file}" <<'PY'
import json
import sys

path = sys.argv[1]
name = sys.argv[2]

with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

unit_scale = {
    "ns": 1,
    "us": 1_000,
    "ms": 1_000_000,
    "s": 1_000_000_000,
}

for bench in payload.get("benchmarks", []):
    if bench.get("run_type") == "aggregate" or bench.get("aggregate_name"):
        continue
    value = bench.get("real_time", bench.get("cpu_time"))
    if value is None:
        continue
    scale = unit_scale.get(bench.get("time_unit", "ns"), 1)
    total_ns = int(round(value * scale))
    print(f'{{"implementation_name":"{name}","total_nanoseconds":{total_ns}}}')
PY
  rm -f "${tmp_json}"
done
