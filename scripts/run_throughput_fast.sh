#!/usr/bin/env bash
# Run approxbench's slim throughput path on the canonical four
# algorithms (kll/cms/countsketch/hll):
#
#   kll-cdf                              k=200
#   hll                                  lg_k=14
#   cms-fastpath-fixedmatrix          rows=5 cols=2048
#   countsketch-fastpath-fixedmatrix  rows=5 cols=2048
#
# Wraps the run in CPU pin / governor / pre-warm logic — single
# core, performance governor, an optional no-turbo lock, and a 2s
# busy-loop pre-warm so schedutil pins the core at max frequency
# before the bench fires.
#
# Usage:
#   scripts/run_throughput_fast.sh                       # all four
#   scripts/run_throughput_fast.sh kll cms               # subset
#   PIN_CORE=2 LOCK_FREQUENCY=1 scripts/run_throughput_fast.sh
#
# Env knobs:
#   PIN_CORE         core to taskset to              (default 2)
#   LOCK_FREQUENCY   1 = `cpupower -g performance`   (default 0)
#   DISABLE_TURBO    1 = echo 1 > intel_pstate/no_turbo (default 0)
#   WARMUP_SECONDS   busy-loop pre-warm before exec  (default 2)
#   RUNS             measured trials per algorithm      (default 10)
#   WARMUP_RUNS      discarded trials per algorithm     (default 2)
#   SIZE             items per trial                 (default 1_000_000)
#   INPUT            override workload file          (default: generate)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

PIN_CORE="${PIN_CORE:-2}"
LOCK_FREQUENCY="${LOCK_FREQUENCY:-0}"
DISABLE_TURBO="${DISABLE_TURBO:-0}"
WARMUP_SECONDS="${WARMUP_SECONDS:-2}"
RUNS="${RUNS:-10}"
WARMUP_RUNS="${WARMUP_RUNS:-2}"
SIZE="${SIZE:-1000000}"
INPUT="${INPUT:-${REPO_ROOT}/input/benchmark_data_1m_int64.bin}"

ALGORITHMS=("$@")
if [[ ${#ALGORITHMS[@]} -eq 0 ]]; then
    ALGORITHMS=(kll cms countsketch hll)
fi

# Canonical (algorithm, impl, --config) tuples, keyed by the short family name
# the caller types. The algorithm the binary is given carries the structural
# variant; the impl is the library.
config_for() {
    case "$1" in
        kll)          echo "kll-cdf|lib|k=200" ;;
        hll)          echo "hll|lib|lg_k=14" ;;
        cms)          echo "cms-fastpath-fixedmatrix|lib|rows=5 cols=2048" ;;
        countsketch)  echo "countsketch-fastpath-fixedmatrix|lib|rows=5 cols=2048" ;;
        *) echo "unknown algorithm: $1" >&2; exit 1 ;;
    esac
}

# Use the plain release binary. The PGO machinery is archived under
# scripts/archive/build_pgo.sh and docs/archive/PGO.md; with
# asap_sketchlib 0.2.2 the inline hint inside the hash dispatch
# delivers the same FixedMatrix specialization without PGO.
BIN_PATH="${REPO_ROOT}/target/release/approxbench"
if [[ ! -x "${BIN_PATH}" ]]; then
    echo "building approxbench (release)..." >&2
    (cd "${REPO_ROOT}" && cargo build --release -p aqpbm-cli >&2)
fi

# --- environment lock-down ---
restore=()
cleanup() {
    for cmd in "${restore[@]}"; do
        eval "$cmd" || true
    done
}
trap cleanup EXIT

if [[ "${DISABLE_TURBO}" == "1" ]]; then
    if [[ -w /sys/devices/system/cpu/intel_pstate/no_turbo ]]; then
        prev=$(cat /sys/devices/system/cpu/intel_pstate/no_turbo)
        echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null
        restore+=("echo $prev | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null")
    else
        echo "warn: cannot toggle intel_pstate/no_turbo" >&2
    fi
fi

if [[ "${LOCK_FREQUENCY}" == "1" ]]; then
    prev_gov=$(cat /sys/devices/system/cpu/cpu${PIN_CORE}/cpufreq/scaling_governor 2>/dev/null || echo "")
    if [[ -n "${prev_gov}" ]]; then
        sudo cpupower -c "${PIN_CORE}" frequency-set -g performance >/dev/null
        restore+=("sudo cpupower -c ${PIN_CORE} frequency-set -g ${prev_gov} >/dev/null")
    fi
fi

sibling_file="/sys/devices/system/cpu/cpu${PIN_CORE}/topology/thread_siblings_list"
if [[ -r "${sibling_file}" ]]; then
    echo "# core=${PIN_CORE} smt_siblings=$(cat ${sibling_file})" >&2
fi

# Build the input flag once. If the canonical 1M file is missing,
# fall back to the CLI's built-in uniform generator at SIZE.
INPUT_FLAG=()
if [[ -f "${INPUT}" ]]; then
    INPUT_FLAG=(--input "${INPUT}")
else
    echo "# input file ${INPUT} not found; using generated uniform workload size=${SIZE}" >&2
    INPUT_FLAG=(--dataset uniform --size "${SIZE}" --cardinality 100000)
fi

run_one() {
    local family="$1"
    local cfg
    cfg=$(config_for "${family}")
    local algorithm="${cfg%%|*}"
    local rest="${cfg#*|}"
    local impl="${rest%%|*}"
    local params="${rest#*|}"

    echo "# === ${algorithm} / ${impl} / ${params} ===" >&2
    # Bash-quoted exec so the pre-warm busy-loop runs on the pinned core.
    taskset -c "${PIN_CORE}" bash -c "
end=\$((SECONDS+${WARMUP_SECONDS}))
while [ \$SECONDS -lt \$end ]; do :; done
exec '${BIN_PATH}' sketchbench \
  --variant '${algorithm}' \
  --library '${impl}' \
  --metrics throughput \
  --config '${params}' \
  --runs ${RUNS} \
  --warmup-runs ${WARMUP_RUNS} \
  $(printf '%q ' "${INPUT_FLAG[@]}")
"
}

for algo in "${ALGORITHMS[@]}"; do
    run_one "${algo}"
done
