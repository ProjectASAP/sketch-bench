#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

run_variant() {
  local VARIANT="$1"
  echo "===== Accuracy: ${VARIANT} ====="

  local VARIANT_DIR="${ACCURACY_DIR}/${VARIANT}"
  local RESULT_PREFIX="${VARIANT}_accuracy"
  local OUTPUT_DIR="${VARIANT_DIR}/output"
  local PLOTS_DIR="${ACCURACY_DIR}/plots/${VARIANT}"
  local SUMMARY_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_results.csv"
  local KEY_ERROR_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors.csv"
  local KEY_SEED_ERROR_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors.csv"
  local RUST_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv"
  local CPP_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv"
  local RUST_KEY_ERROR_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_rust.csv"
  local CPP_KEY_ERROR_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_cpp.csv"
  local RUST_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_rust.csv"
  local CPP_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cpp.csv"
  local RUST_CAIDA_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida_rust.csv"
  local CPP_CAIDA_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida_cpp.csv"
  local CAIDA_SOURCE_PCAP="${ACCURACY_CMS_CAIDA_SOURCE_PCAP:-${ACCURACY_DIR}/../input/equinix-nyc.dirA.20190117-125910.UTC.anon.pcap}"
  local CAIDA_KEY_SEED_CSV="${ACCURACY_CMS_CAIDA_KEY_SEED_ERRORS:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_caida.csv}"
  local CIC_INPUT_DIR="${ACCURACY_DIR}/../input"
  local CIC_SENTINEL="${CIC_INPUT_DIR}/Friday-WorkingHours-Afternoon-DDos.pcap_ISCX.csv"
  local CIC_COMBINED_CSV="${OUTPUT_DIR}/cic_combined.csv"
  local RUST_CIC_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cic_rust.csv"
  local CPP_CIC_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cic_cpp.csv"
  local CIC_KEY_SEED_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cic.csv"
  local DATASET_COMPARE_COLS="${ACCURACY_CMS_DATASET_COMPARE_COLS:-65536}"
  local DATASET_COMPARE_SEED="${ACCURACY_CMS_DATASET_COMPARE_SEED:-5}"
  local DATASET_COMPARE_PLOT_PATH="${PLOTS_DIR}/${RESULT_PREFIX}_dataset_compare_col_${DATASET_COMPARE_COLS}_seed_${DATASET_COMPARE_SEED}.png"
  mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

  if [[ "${VARIANT}" == "octo" ]]; then
    "${SCRIPT_DIR}/run_accuracy_rust.sh" "" "" "${VARIANT}"

    python3 "${SCRIPT_DIR}/plot_octo_accuracy.py" \
      --input-cms "${OUTPUT_DIR}/octo_accuracy_cms.csv" \
      --input-cs "${OUTPUT_DIR}/octo_accuracy_cs.csv" \
      --input-hll "${OUTPUT_DIR}/octo_accuracy_hll.csv" \
      --output-cms "${PLOTS_DIR}/octo_accuracy_cms.png" \
      --output-cs "${PLOTS_DIR}/octo_accuracy_cs.png" \
      --output-hll "${PLOTS_DIR}/octo_accuracy_hll.png"

    echo "Wrote ${PLOTS_DIR}/octo_accuracy_cms.png"
    echo "Wrote ${PLOTS_DIR}/octo_accuracy_cs.png"
    echo "Wrote ${PLOTS_DIR}/octo_accuracy_hll.png"
  elif [[ "${VARIANT}" == "kll" ]]; then
    local SUMMARY_HEADER="implementation,language,k,percentile,total_items,true_quantile,estimate,relative_error"
    "${SCRIPT_DIR}/run_accuracy_rust.sh" "${RUST_SUMMARY_CSV}" "" "${VARIANT}"
    "${SCRIPT_DIR}/run_accuracy_cpp.sh" "${CPP_SUMMARY_CSV}" "" "${VARIANT}"

    {
      echo "${SUMMARY_HEADER}"
      tail -n +2 "${RUST_SUMMARY_CSV}"
      cat "${CPP_SUMMARY_CSV}"
    } > "${SUMMARY_CSV_PATH}"

    python3 "${SCRIPT_DIR}/plot_kll_accuracy.py" \
      --input "${SUMMARY_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_relative_error.png"

    echo "Wrote ${SUMMARY_CSV_PATH}"
    echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_relative_error.png"
  elif [[ "${VARIANT}" == "hll" ]]; then
    local SUMMARY_HEADER="implementation,language,seed,lg_k,registers,total_items,true_distinct,estimate,relative_error"
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
    local SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error"
    local KEY_ERROR_HEADER="implementation,language,rows,cols,key,true_count,median_estimate,median_relative_error"
    local KEY_SEED_ERROR_HEADER="implementation,language,seed,rows,cols,key,true_count,estimate,relative_error"

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

    if [[ "${VARIANT}" == "cms" ]]; then
      {
        echo "${KEY_SEED_ERROR_HEADER}"
        tail -n +2 "${RUST_KEY_SEED_CSV}"
        cat "${CPP_KEY_SEED_CSV}"
      } > "${KEY_SEED_ERROR_CSV_PATH}"
    fi

    python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
      --variant "${VARIANT}" \
      --input-key-errors "${KEY_ERROR_CSV_PATH}" \
      --output "${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error.png"

    if [[ "${VARIANT}" == "cms" ]]; then
      local DATASET_ARGS=("--dataset-key-seed-errors" "zipf=${KEY_SEED_ERROR_CSV_PATH}")

      if [[ -f "${CAIDA_SOURCE_PCAP}" ]]; then
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

        DATASET_ARGS+=("--dataset-key-seed-errors" "caida=${CAIDA_KEY_SEED_CSV}")
        echo "Wrote ${CAIDA_KEY_SEED_CSV}"
      fi

      if [[ -f "${CIC_SENTINEL}" ]]; then
        {
          head -1 "${CIC_SENTINEL}"
          for f in "${CIC_INPUT_DIR}"/Friday-*.csv "${CIC_INPUT_DIR}"/Monday-*.csv \
                   "${CIC_INPUT_DIR}"/Thursday-*.csv "${CIC_INPUT_DIR}"/Tuesday-*.csv \
                   "${CIC_INPUT_DIR}"/Wednesday-*.csv; do
            tail -n +2 "$f"
          done
        } > "${CIC_COMBINED_CSV}"

        cargo run --release --offline --manifest-path "${VARIANT_DIR}/rust/Cargo.toml" -- \
          --data "${CIC_COMBINED_CSV}" \
          --output-key-seed-errors "${RUST_CIC_KEY_SEED_CSV}" \
          --skip-summary \
          --skip-key-errors \
          --seed "${DATASET_COMPARE_SEED}" \
          --cols "${DATASET_COMPARE_COLS}"

        "${VARIANT_DIR}/cpp/build/cms_accuracy" \
          --data "${CIC_COMBINED_CSV}" \
          --seed "${DATASET_COMPARE_SEED}" \
          --cols "${DATASET_COMPARE_COLS}" \
          --mode key-seed-errors \
          > "${CPP_CIC_KEY_SEED_CSV}"

        {
          echo "${KEY_SEED_ERROR_HEADER}"
          tail -n +2 "${RUST_CIC_KEY_SEED_CSV}"
          cat "${CPP_CIC_KEY_SEED_CSV}"
        } > "${CIC_KEY_SEED_CSV}"

        DATASET_ARGS+=("--dataset-key-seed-errors" "cic=${CIC_KEY_SEED_CSV}")
        echo "Wrote ${CIC_KEY_SEED_CSV}"
      fi

      if [[ ${#DATASET_ARGS[@]} -ge 4 ]]; then
        python3 "${SCRIPT_DIR}/plot_cms_accuracy.py" \
          --variant "${VARIANT}" \
          --input-key-errors "${KEY_ERROR_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_avg_relative_error.png" \
          "${DATASET_ARGS[@]}" \
          --dataset-boxplot-cols "${DATASET_COMPARE_COLS}" \
          --dataset-boxplot-seed "${DATASET_COMPARE_SEED}" \
          --dataset-boxplot-output "${DATASET_COMPARE_PLOT_PATH}"
        echo "Wrote ${DATASET_COMPARE_PLOT_PATH}"
      fi
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
  fi
}

REQUESTED="${1:-${ACCURACY_VARIANT:-all}}"

case "${REQUESTED}" in
  cms|cs|hll|kll|octo)
    run_variant "${REQUESTED}"
    ;;
  all)
    for v in cms cs hll kll octo; do
      run_variant "$v"
    done
    ;;
  *)
    echo "unsupported variant: ${REQUESTED}; expected cms, cs, hll, kll, octo, or all" >&2
    exit 1
    ;;
esac
