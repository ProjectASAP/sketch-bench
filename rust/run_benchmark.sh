#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DATA_FILE="${ROOT_DIR}/input/benchmark_data_1m_int64.bin"

echo "========================================"
echo "bench_sketches Rust Benchmark Runner"
echo "========================================"
echo ""

cd "$SCRIPT_DIR"
mkdir -p output

if [[ ! -f "${DATA_FILE}" ]]; then
  echo "Input data not found at ${DATA_FILE}. Generating..."
  "${ROOT_DIR}/input/generate_data.sh"
fi

PIN_CORE="${PIN_CORE:-0}"
LOCK_FREQUENCY="${LOCK_FREQUENCY:-0}"
DISABLE_LTO="${DISABLE_LTO:-0}"
LTO_MODE="${LTO_MODE:-fat}"

BINARIES=(
  hll_oxide
  hll_lib
  countmin_oxide
  countmin_lib_vector2d_fast
  countmin_lib_vector2d_regular
  countmin_lib_fixedmatrix_fast
  countmin_lib_fixedmatrix_custom_fast
  countsketch_oxide
  countsketch_lib_vector2d_fast
  countsketch_lib_vector2d_regular
  countsketch_lib_fixedmatrix_fast
  elastic_oxide
  elastic_lib
  kll_oxide
  kll_lib
  univmon_oxide
  univmon_lib
  nitro_oxide
  nitro_lib
  hll_datasketches
  countmin_datasketches
)

taskset_prefix=()
if command -v taskset >/dev/null 2>&1; then
  taskset_prefix=(taskset -c "${PIN_CORE}")
  echo "CPU pinning enabled via taskset on core ${PIN_CORE}"
else
  echo "taskset not available; CPU pinning skipped."
fi

lock_frequency() {
  if [[ "${LOCK_FREQUENCY}" != "1" ]]; then
    echo "CPU frequency locking skipped (set LOCK_FREQUENCY=1 to attempt)."
    return
  fi

  if command -v cpupower >/dev/null 2>&1; then
    echo "Setting CPU governor to performance (requires sudo)..."
    sudo -n cpupower frequency-set -g performance || echo "cpupower frequency-set failed; check permissions."
  else
    echo "cpupower not available; cannot set governor."
  fi

  if [[ -w /sys/devices/system/cpu/intel_pstate/no_turbo ]]; then
    echo "Disabling Turbo Boost (requires sudo)..."
    echo "1" | sudo -n tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null || echo "Turbo disable failed; check permissions."
  else
    echo "Turbo control interface not available on this host."
  fi
}

echo "Locking CPU frequency controls (if enabled)..."
lock_frequency
echo ""

echo "Building benchmark binaries..."
rustflags_base="-C target-cpu=native -C opt-level=3 -C codegen-units=1"
if [[ "${DISABLE_LTO}" != "1" ]]; then
  rustflags_base="${rustflags_base} -C embed-bitcode=yes"
fi
export RUSTFLAGS="${rustflags_base} ${RUSTFLAGS:-}"
if [[ "${DISABLE_LTO}" == "1" ]]; then
  export CARGO_PROFILE_RELEASE_LTO=off
else
  export CARGO_PROFILE_RELEASE_LTO="${LTO_MODE}"
fi
cargo build --release --bins
echo "Build complete."
echo ""

echo "Running benchmark suite..."
for bin in "${BINARIES[@]}"; do
  echo "----------------------------------------"
  echo "Benchmark: ${bin}"
  echo "----------------------------------------"
  output_file="output/${bin}.jsonl"
  "${taskset_prefix[@]}" "target/release/${bin}" | tee "${output_file}"
done
echo ""

echo "========================================"
echo "bench_sketches suite finished"
echo "Results saved to rust/output/"
echo "========================================"
