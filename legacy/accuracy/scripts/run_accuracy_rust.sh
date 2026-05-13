#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${3:-${ACCURACY_VARIANT:-cms}}"

# Aliases map legacy per-sketch names onto the statistic-based
# accuracy crates (cardinality/, frequency/, quantile/) that
# mirror `sketch-bench/src/baselines/*`.
#
# Legacy      → crate         sketch arg
#   hll       → cardinality   (HLL is the only cardinality sketch)
#   cms, cs   → frequency     --sketch cms / --sketch countsketch
#   kll, dd   → quantile      --sketch kll / --sketch dd
CRATE_DIR=""
SKETCH_ARG=""
case "${VARIANT}" in
  hll|cardinality)
    CRATE_DIR="${ACCURACY_DIR}/cardinality"
    ;;
  cms)
    CRATE_DIR="${ACCURACY_DIR}/frequency"
    SKETCH_ARG="cms"
    ;;
  cs|countsketch)
    CRATE_DIR="${ACCURACY_DIR}/frequency"
    SKETCH_ARG="countsketch"
    ;;
  kll)
    CRATE_DIR="${ACCURACY_DIR}/quantile"
    SKETCH_ARG="kll"
    ;;
  dd)
    CRATE_DIR="${ACCURACY_DIR}/quantile"
    SKETCH_ARG="dd"
    ;;
  nitro|octo)
    CRATE_DIR="${ACCURACY_DIR}/${VARIANT}"
    ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cardinality (hll), cms, cs (countsketch), kll, dd, nitro, or octo" >&2
    exit 1
    ;;
esac

RESULT_PREFIX="${VARIANT}_accuracy"
OUTPUT_DIR="${CRATE_DIR}/output"
DATA_DST="${4:-${ACCURACY_DATASET:-${ACCURACY_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin}}"
SUMMARY_OUTPUT_PATH="${1:-${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv}"
KEY_SEED_ERROR_OUTPUT_PATH="${ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_rust.csv}"
export TMPDIR="${ACCURACY_DIR}/.tmp"
export CARGO_TARGET_DIR="/tmp/sketchlib-bench-accuracy-target/${VARIANT}"

mkdir -p "${ACCURACY_DIR}/../input" "${OUTPUT_DIR}" "${TMPDIR}" "${CARGO_TARGET_DIR}"

if [[ ! -f "${DATA_DST}" ]]; then
  if [[ "${DATA_DST}" == "${ACCURACY_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin" ]]; then
    "${ACCURACY_DIR}/../input/generate_zipf_data.sh"
  else
    echo "dataset not found: ${DATA_DST}" >&2
    exit 1
  fi
fi

if [[ ! -f "${CRATE_DIR}/rust/Cargo.lock" ]]; then
  cargo generate-lockfile --offline --manifest-path "${CRATE_DIR}/rust/Cargo.toml"
fi

# Build the --sketch flag if this crate expects one (currently the
# unified `frequency` crate). Leave empty for solo-sketch crates.
SKETCH_FLAG=()
if [[ -n "${SKETCH_ARG}" ]]; then
  SKETCH_FLAG=(--sketch "${SKETCH_ARG}")
fi

if [[ "${VARIANT}" == "octo" ]]; then
  cargo run --release --offline --manifest-path "${CRATE_DIR}/rust/Cargo.toml" -- \
    --data "${DATA_DST}" \
    --output-cms "${OUTPUT_DIR}/octo_accuracy_cms.csv" \
    --output-cs "${OUTPUT_DIR}/octo_accuracy_cs.csv" \
    --output-hll "${OUTPUT_DIR}/octo_accuracy_hll.csv"
elif [[ "${VARIANT}" == "cardinality" || "${VARIANT}" == "hll" || "${VARIANT}" == "kll" || "${VARIANT}" == "dd" || "${VARIANT}" == "nitro" ]]; then
  cargo run --release --offline --manifest-path "${CRATE_DIR}/rust/Cargo.toml" -- \
    "${SKETCH_FLAG[@]}" \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}"
elif [[ "${VARIANT}" == "cms" ]]; then
  KEY_ERROR_OUTPUT_PATH="${2:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_rust.csv}"
  cargo run --release --offline --manifest-path "${CRATE_DIR}/rust/Cargo.toml" -- \
    "${SKETCH_FLAG[@]}" \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}" \
    --output-key-errors "${KEY_ERROR_OUTPUT_PATH}" \
    --output-key-seed-errors "${KEY_SEED_ERROR_OUTPUT_PATH}"
else
  # cs / countsketch
  KEY_ERROR_OUTPUT_PATH="${2:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_rust.csv}"
  cargo run --release --offline --manifest-path "${CRATE_DIR}/rust/Cargo.toml" -- \
    "${SKETCH_FLAG[@]}" \
    --data "${DATA_DST}" \
    --output-summary "${SUMMARY_OUTPUT_PATH}" \
    --output-key-errors "${KEY_ERROR_OUTPUT_PATH}" \
    --skip-key-seed-errors
fi
