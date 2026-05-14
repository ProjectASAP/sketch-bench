#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${THROUGHPUT_VARIANT:-cms}}"
OUTPUT_ARG="${2:-}"
OP="${3:-${THROUGHPUT_OP:-insert}}"

case "${VARIANT}" in
  cms|cs|hll|kll|dd|polars-freq|polars-quantile|polars-cardinality) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, hll, kll, dd, polars-freq, polars-quantile, or polars-cardinality" >&2
    exit 1
    ;;
esac

case "${OP}" in
  insert|query) ;;
  *)
    echo "unsupported op: ${OP}; expected insert or query" >&2
    exit 1
    ;;
esac

DEFAULT_DATASET="${THROUGHPUT_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin"
DATA_DST="${THROUGHPUT_DATASET:-${ACCURACY_DATASET:-${DEFAULT_DATASET}}}"

case "${VARIANT}" in
  cms|cs)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_freq"
    POLARS_BIN_PREFIX="polars_freq_throughput"
    POLARS_VARIANT="${VARIANT}"
    RESULT_STEM="${VARIANT}"
    ;;
  polars-freq)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_freq"
    POLARS_BIN_PREFIX="polars_freq_throughput"
    POLARS_VARIANT="cms"
    RESULT_STEM="polars_freq"
    ;;
  kll|dd)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_quantile"
    POLARS_BIN_PREFIX="polars_quantile_throughput"
    POLARS_VARIANT="${VARIANT}"
    RESULT_STEM="${VARIANT}"
    ;;
  polars-quantile)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_quantile"
    POLARS_BIN_PREFIX="polars_quantile_throughput"
    POLARS_VARIANT="kll"
    RESULT_STEM="polars_quantile"
    ;;
  hll)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_cardinality"
    POLARS_BIN_PREFIX="polars_cardinality_throughput"
    POLARS_VARIANT="${VARIANT}"
    RESULT_STEM="${VARIANT}"
    ;;
  polars-cardinality)
    POLARS_DIR="${THROUGHPUT_DIR}/polars_cardinality"
    POLARS_BIN_PREFIX="polars_cardinality_throughput"
    POLARS_VARIANT="hll"
    RESULT_STEM="polars_cardinality"
    ;;
  *)
    echo "polars variant not present for ${VARIANT}; skipping" >&2
    exit 0
    ;;
esac

OUTPUT_DIR="${POLARS_DIR}/output"

if [[ "${OP}" == "insert" ]]; then
  BIN_NAME="${POLARS_BIN_PREFIX}"
  if [[ "${VARIANT}" == "polars-freq" || "${VARIANT}" == "polars-quantile" || "${VARIANT}" == "polars-cardinality" ]]; then
    DEFAULT_OUTPUT="${OUTPUT_DIR}/${RESULT_STEM}_throughput_results.csv"
  else
    DEFAULT_OUTPUT="${OUTPUT_DIR}/${RESULT_STEM}_throughput_results_polars.csv"
  fi
else
  BIN_NAME="${POLARS_BIN_PREFIX}_query"
  if [[ "${VARIANT}" == "polars-freq" || "${VARIANT}" == "polars-quantile" || "${VARIANT}" == "polars-cardinality" ]]; then
    DEFAULT_OUTPUT="${OUTPUT_DIR}/${RESULT_STEM}_query_results.csv"
  else
    DEFAULT_OUTPUT="${OUTPUT_DIR}/${RESULT_STEM}_throughput_query_results_polars.csv"
  fi
fi

OUTPUT_PATH="${OUTPUT_ARG:-${DEFAULT_OUTPUT}}"

mkdir -p "${THROUGHPUT_DIR}/../input" "${OUTPUT_DIR}"

if [[ "${DATA_DST}" == "${DEFAULT_DATASET}" && ! -f "${DATA_DST}" ]]; then
  "${THROUGHPUT_DIR}/../input/generate_zipf_data.sh"
elif [[ ! -f "${DATA_DST}" ]]; then
  echo "dataset not found: ${DATA_DST}" >&2
  exit 1
fi

if [[ ! -f "${POLARS_DIR}/Cargo.lock" ]]; then
  cargo generate-lockfile --manifest-path "${POLARS_DIR}/Cargo.toml"
fi

cargo run --release --manifest-path "${POLARS_DIR}/Cargo.toml" --bin "${BIN_NAME}" -- \
  --data "${DATA_DST}" \
  --variant "${POLARS_VARIANT}" \
  --output "${OUTPUT_PATH}"
