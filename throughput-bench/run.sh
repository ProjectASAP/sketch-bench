#!/usr/bin/env bash
set -euo pipefail

PIN_CORE="${PIN_CORE:-2}"
LOCK_FREQUENCY="${LOCK_FREQUENCY:-0}"
DISABLE_TURBO="${DISABLE_TURBO:-0}"
DROP_CACHES="${DROP_CACHES:-0}"

if [[ $# -lt 1 ]]; then
    echo "usage: PIN_CORE=2 LOCK_FREQUENCY=1 $0 <bin-name> [args...]" >&2
    echo "  binaries: $(ls target/release/ 2>/dev/null | grep -v '\.' | tr '\n' ' ')" >&2
    exit 1
fi

BIN="$1"
shift

BIN_PATH="target/release/${BIN}"
if [[ ! -x "$BIN_PATH" ]]; then
    echo "building $BIN..." >&2
    cargo build --release --bin "$BIN" >&2
fi

restore=()
cleanup() {
    for cmd in "${restore[@]}"; do
        eval "$cmd" || true
    done
}
trap cleanup EXIT

if [[ "$DISABLE_TURBO" == "1" ]]; then
    if [[ -w /sys/devices/system/cpu/intel_pstate/no_turbo ]]; then
        prev=$(cat /sys/devices/system/cpu/intel_pstate/no_turbo)
        echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null
        restore+=("echo $prev | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null")
    else
        echo "warn: cannot toggle intel_pstate/no_turbo" >&2
    fi
fi

if [[ "$LOCK_FREQUENCY" == "1" ]]; then
    prev_gov=$(cat /sys/devices/system/cpu/cpu${PIN_CORE}/cpufreq/scaling_governor 2>/dev/null || echo "")
    if [[ -n "$prev_gov" ]]; then
        sudo cpupower -c "$PIN_CORE" frequency-set -g performance >/dev/null
        restore+=("sudo cpupower -c $PIN_CORE frequency-set -g $prev_gov >/dev/null")
    fi
fi

if [[ "$DROP_CACHES" == "1" ]]; then
    sync
    echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null
fi

# Show the SMT sibling so the user knows what else might contend
sibling_file="/sys/devices/system/cpu/cpu${PIN_CORE}/topology/thread_siblings_list"
if [[ -r "$sibling_file" ]]; then
    echo "# core=$PIN_CORE smt_siblings=$(cat $sibling_file)" >&2
fi

exec taskset -c "$PIN_CORE" "./$BIN_PATH" "$@"
