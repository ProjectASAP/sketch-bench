#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${1:-${ACCURACY_VARIANT:-cms}}"

case "${VARIANT}" in
  cms|cs|hll) ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cms, cs, or hll" >&2
    exit 1
    ;;
esac

VARIANT_DIR="${ACCURACY_DIR}/${VARIANT}"
RESULT_PREFIX="${VARIANT}_accuracy"
OUTPUT_DIR="${VARIANT_DIR}/output"
PLOTS_DIR="${ACCURACY_DIR}/plots/${VARIANT}"
SUMMARY_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_results.csv"
KEY_ERROR_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors.csv"
KEY_SEED_ERROR_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors.csv"
RUST_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv"
CPP_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv"
RUST_KEY_ERROR_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_rust.csv"
CPP_KEY_ERROR_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_cpp.csv"
RUST_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_rust.csv"
CPP_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cpp.csv"
RUST_CAIDA_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida_rust.csv"
CPP_CAIDA_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida_cpp.csv"
CAIDA_SOURCE_PCAP="${ACCURACY_CMS_CAIDA_SOURCE_PCAP:-${ACCURACY_DIR}/../input/equinix-nyc.dirA.20190117-125910.UTC.anon.pcap}"
CAIDA_KEY_SEED_CSV="${ACCURACY_CMS_CAIDA_KEY_SEED_ERRORS:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida.csv}"
DATASET_COMPARE_COLS="${ACCURACY_CMS_DATASET_COMPARE_COLS:-65536}"
DATASET_COMPARE_SEED="${ACCURACY_CMS_DATASET_COMPARE_SEED:-5}"
DATASET_COMPARE_PLOT_PATH="${PLOTS_DIR}/${RESULT_PREFIX}_dataset_compare_col_${DATASET_COMPARE_COLS}_seed_${DATASET_COMPARE_SEED}.png"
mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

if [[ "${VARIANT}" == "hll" ]]; then
  SUMMARY_HEADER="implementation,language,seed,lg_k,registers,total_items,true_distinct,estimate,relative_error"
  "${SCRIPT_DIR}/run_accuracy_rust.sh" "${RUST_SUMMARY_CSV}" "" "${VARIANT}"
  "${SCRIPT_DIR}/run_accuracy_cpp.sh" "${CPP_SUMMARY_CSV}" "" "${VARIANT}"

  {
    echo "${SUMMARY_HEADER}"
    tail -n +2 "${RUST_SUMMARY_CSV}"
    cat "${CPP_SUMMARY_CSV}"
  } > "${SUMMARY_CSV_PATH}"

  python3 "${SCRIPT_DIR}/plot_hll_accuracy.py" \
    --input "${SUMMARY_CSV_PATH}" \
    --output "${PLOTS_DIR}/${RESULT_PREFIX}_relative_error.png"

  echo "Wrote ${SUMMARY_CSV_PATH}"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_relative_error.png"
else
  SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error"
  KEY_ERROR_HEADER="implementation,language,rows,cols,key,true_count,median_estimate,median_relative_error"
  KEY_SEED_ERROR_HEADER="implementation,language,seed,rows,cols,key,true_count,estimate,relative_error"

  ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH="${RUST_KEY_SEED_CSV}" \
    "${SCRIPT_DIR}/run_accuracy_rust.sh" "${RUST_SUMMARY_CSV}" "${RUST_KEY_ERROR_CSV}" "${VARIANT}"
  ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH="${CPP_KEY_SEED_CSV}" \
    "${SCRIPT_DIR}/run_accuracy_cpp.sh" "${CPP_SUMMARY_CSV}" "${CPP_KEY_ERROR_CSV}" "${VARIANT}"

  {
    echo "${SUMMARY_HEADER}"
    tail -n +2 "${RUST_SUMMARY_CSV}"
    cat "${CPP_SUMMARY_CSV}"
  } > "${SUMMARY_CSV_PATH}"

  {
    echo "${KEY_ERROR_HEADER}"
    tail -n +2 "${RUST_KEY_ERROR_CSV}"
    cat "${CPP_KEY_ERROR_CSV}"
  } > "${KEY_ERROR_CSV_PATH}"

  {
    echo "${KEY_SEED_ERROR_HEADER}"
    tail -n +2 "${RUST_KEY_SEED_CSV}"
    cat "${CPP_KEY_SEED_CSV}"
  } > "${KEY_SEED_ERROR_CSV_PATH}"

  python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
    --variant "${VARIANT}" \
    --input-key-errors "${KEY_ERROR_CSV_PATH}" \
    --output "${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error.png"

  if [[ "${VARIANT}" == "cms" && -f "${CAIDA_SOURCE_PCAP}" ]]; then
    cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
      --data "${CAIDA_SOURCE_PCAP}" \
      --output-key-seed-errors "${RUST_CAIDA_KEY_SEED_CSV}" \
      --skip-summary \
      --skip-key-errors \
      --seed "${DATASET_COMPARE_SEED}" \
      --cols "${DATASET_COMPARE_COLS}"

    "${VARIANT_DIR}/cpp/build/cms_accuracy" \
      --data "${CAIDA_SOURCE_PCAP}" \
      --seed "${DATASET_COMPARE_SEED}" \
      --cols "${DATASET_COMPARE_COLS}" \
      --mode key-seed-errors \
      > "${CPP_CAIDA_KEY_SEED_CSV}"

    {
      echo "${KEY_SEED_ERROR_HEADER}"
      tail -n +2 "${RUST_CAIDA_KEY_SEED_CSV}"
      cat "${CPP_CAIDA_KEY_SEED_CSV}"
    } > "${CAIDA_KEY_SEED_CSV}"

    python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
      --variant "${VARIANT}" \
      --input-key-errors "${KEY_ERROR_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error.png" \
      --dataset-key-seed-errors "zipf=${KEY_SEED_ERROR_CSV_PATH}" \
      --dataset-key-seed-errors "caida=${CAIDA_KEY_SEED_CSV}" \
      --dataset-boxplot-cols "${DATASET_COMPARE_COLS}" \
      --dataset-boxplot-seed "${DATASET_COMPARE_SEED}" \
      --dataset-boxplot-output "${DATASET_COMPARE_PLOT_PATH}"
  fi

  echo "Wrote ${SUMMARY_CSV_PATH}"
  echo "Wrote ${KEY_ERROR_CSV_PATH}"
  echo "Wrote ${KEY_SEED_ERROR_CSV_PATH}"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_from_8192.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_from_16384.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_2048.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_4096.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_8192.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_16384.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_32768.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_65536.png"
  echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error_col_131072.png"
  if [[ "${VARIANT}" == "cms" && -f "${CAIDA_SOURCE_PCAP}" ]]; then
    echo "Wrote ${CAIDA_KEY_SEED_CSV}"
    echo "Wrote ${DATASET_COMPARE_PLOT_PATH}"
  fi
fi
