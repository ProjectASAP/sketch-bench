#!/usr/bin/env bash
# Build a PGO-optimised sketchlib binary end-to-end. Wraps the 4
# stages described in docs/PGO.md so users on a different machine
# don't have to remember the magic.
#
# Stages:
#   1. instrumented build  (-Cprofile-generate)
#   2. profile run         (workload binary; either a user file or
#                           the default 10M zipf, generated if missing)
#   3. merge               (llvm-profdata merge)
#   4. optimised build     (-Cprofile-use) → target/release-pgo/sketchlib
#
# Output path note: stage 4 deliberately writes to target/release-pgo/
# rather than target/release/. cargo owns target/release/, so any later
# `cargo build --release -p aqpbm-cli` (without --target) silently
# overwrites the PGO binary with a fresh non-PGO build, and downstream
# scripts then run ~30–60 % slower with no visible signal. Keeping the
# PGO artefact in a path cargo never touches makes that footgun
# impossible. scripts/run_throughput_fast.sh prefers the PGO binary and
# falls back to target/release/ if it's missing.
#
# Usage:
#   scripts/build_pgo.sh                                 # default workload
#   scripts/build_pgo.sh path/to/my_workload.bin         # custom workload
#   PIN_CORE=2 BENCH_WARMUP_SECS=10 scripts/build_pgo.sh
#
# Env knobs:
#   PIN_CORE           core to taskset profile run to     (default 2)
#   BENCH_WARMUP_SECS  CPU pre-warm before profile run    (default 10)
#   PGO_DIR            where to drop .profraw files       (default target/pgo-profiles)
#   PROFDATA_OUT       merged profile path                (default target/pgo-merged.profdata)
#
# Requirements:
#   rustup component add llvm-tools-preview      # one-time
#   g++ (only if the default workload needs generating)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PIN_CORE="${PIN_CORE:-2}"
export BENCH_WARMUP_SECS="${BENCH_WARMUP_SECS:-10}"
PGO_DIR="${PGO_DIR:-${REPO_ROOT}/target/pgo-profiles}"
PROFDATA_OUT="${PROFDATA_OUT:-${REPO_ROOT}/target/pgo-merged.profdata}"

DEFAULT_WORKLOAD="${REPO_ROOT}/input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
WORKLOAD="${1:-${DEFAULT_WORKLOAD}}"

# --- prereq: llvm-profdata ------------------------------------------------
HOST_TRIPLE="$(rustc -vV | awk '/host/{print $2}')"
TOOLCHAIN_DIR="$(rustc --print sysroot)"
PROFDATA="${TOOLCHAIN_DIR}/lib/rustlib/${HOST_TRIPLE}/bin/llvm-profdata"
if [[ ! -x "${PROFDATA}" ]]; then
    echo "error: llvm-profdata not found at ${PROFDATA}" >&2
    echo "       run: rustup component add llvm-tools-preview" >&2
    exit 1
fi
echo "# llvm-profdata: ${PROFDATA}" >&2
echo "# host triple:   ${HOST_TRIPLE}" >&2

# --- prereq: workload -----------------------------------------------------
if [[ ! -f "${WORKLOAD}" ]]; then
    if [[ "${WORKLOAD}" != "${DEFAULT_WORKLOAD}" ]]; then
        echo "error: workload file not found: ${WORKLOAD}" >&2
        exit 1
    fi
    echo "# default workload missing — generating ${WORKLOAD}" >&2
    if ! command -v g++ >/dev/null 2>&1; then
        echo "error: g++ needed to build the zipf generator, not on PATH" >&2
        exit 1
    fi
    # The script names the output from its args; pass exactly the
    # defaults that produce the canonical filename above.
    (cd "${REPO_ROOT}/input" && bash generate_zipf_data.sh 10000000 1.1 100000 42)
    if [[ ! -f "${WORKLOAD}" ]]; then
        echo "error: generator finished but ${WORKLOAD} still missing" >&2
        exit 1
    fi
fi
echo "# workload: ${WORKLOAD}" >&2

# --- stage 1: instrumented build -----------------------------------------
rm -rf "${PGO_DIR}"
mkdir -p "${PGO_DIR}"
echo "# === stage 1: instrumented build ===" >&2
RUSTFLAGS="-C target-cpu=native -Cprofile-generate=${PGO_DIR}" \
    cargo build --release -p aqpbm-cli \
        --target "${HOST_TRIPLE}" >&2

INSTRUMENTED_BIN="${REPO_ROOT}/target/${HOST_TRIPLE}/release/sketchlib"

# --- stage 2: profile run ------------------------------------------------
echo "# === stage 2: profile run (workload=$(basename "${WORKLOAD}")) ===" >&2
# Drive every hot path we care about (the 4 throughput families).
# bench --metrics throughput is enough — accuracy / latency paths
# don't have a binary-level specialization problem worth profiling.
for tuple in \
    "kll|lib|k=200" \
    "hll|lib|lg_k=14" \
    "cms|lib-fixedmatrix-fast|rows=5 cols=2048" \
    "countsketch|lib-fixedmatrix-fast|rows=5 cols=2048" ; do
    family="${tuple%%|*}"
    rest="${tuple#*|}"
    impl="${rest%%|*}"
    cfg="${rest#*|}"
    echo "#   profiling ${family}/${impl} ${cfg}" >&2
    taskset -c "${PIN_CORE}" "${INSTRUMENTED_BIN}" bench \
        --sketch "${family}" --impl "${impl}" \
        --metrics throughput \
        --config "${cfg}" \
        --runs 3 --warmup-runs 1 \
        --input "${WORKLOAD}" >/dev/null
done

profraw_count="$(find "${PGO_DIR}" -name '*.profraw' | wc -l)"
if [[ "${profraw_count}" -eq 0 ]]; then
    echo "error: no .profraw files in ${PGO_DIR} — instrumented binary did not emit profile data" >&2
    exit 2
fi
echo "# collected ${profraw_count} .profraw file(s)" >&2

# --- stage 3: merge -------------------------------------------------------
echo "# === stage 3: merge profiles ===" >&2
"${PROFDATA}" merge -o "${PROFDATA_OUT}" "${PGO_DIR}"
echo "# merged profile: ${PROFDATA_OUT} ($(stat -c %s "${PROFDATA_OUT}") bytes)" >&2

# --- stage 4: optimised build --------------------------------------------
echo "# === stage 4: optimised build ===" >&2
RUSTFLAGS="-C target-cpu=native -Cprofile-use=${PROFDATA_OUT}" \
    cargo build --release -p aqpbm-cli \
        --target "${HOST_TRIPLE}" >&2

# Copy into target/release-pgo/ — a path cargo never writes to, so a
# later `cargo build --release` can't silently clobber the PGO binary.
# scripts/run_throughput_fast.sh prefers this path.
PGO_BIN_DIR="${REPO_ROOT}/target/release-pgo"
mkdir -p "${PGO_BIN_DIR}"
cp "${REPO_ROOT}/target/${HOST_TRIPLE}/release/sketchlib" \
   "${PGO_BIN_DIR}/sketchlib"

size_bytes="$(stat -c %s "${PGO_BIN_DIR}/sketchlib")"
echo "# done: target/release-pgo/sketchlib (${size_bytes} bytes)" >&2
