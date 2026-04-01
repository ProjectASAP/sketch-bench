#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THROUGHPUT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

run_variant() {
  local VARIANT="$1"
  echo "===== Throughput: ${VARIANT} ====="

  local VARIANT_DIR="${THROUGHPUT_DIR}/${VARIANT}"
  local RESULT_PREFIX="${VARIANT}_throughput"
  local OUTPUT_DIR="${VARIANT_DIR}/output"
  local PLOTS_DIR="${THROUGHPUT_DIR}/plots/${VARIANT}"
  local SUMMARY_CSV_PATH="${OUTPUT_DIR}/${RESULT_PREFIX}_results.csv"
  local RUST_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_rust.csv"
  local CPP_SUMMARY_CSV="${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv"
  local SUMMARY_HEADER
  case "${VARIANT}" in
    kll)
      SUMMARY_HEADER="implementation,language,run,k,total_items,total_nanoseconds,throughput_items_per_sec"
      ;;
    hll)
      SUMMARY_HEADER="implementation,language,run,lg_k,registers,total_items,total_nanoseconds,throughput_items_per_sec"
      ;;
    *)
      SUMMARY_HEADER="implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec"
      ;;
  esac

  local RUST_ONLY=false
  case "${VARIANT}" in
    octo|cs32k) RUST_ONLY=true ;;
  esac

  mkdir -p "${OUTPUT_DIR}" "${PLOTS_DIR}"

  if [[ "${VARIANT}" == "octo" ]]; then
    "${SCRIPT_DIR}/run_throughput_rust.sh" "${VARIANT}" "${RUST_SUMMARY_CSV}"

    python3 "${SCRIPT_DIR}/plot_octo_throughput.py" \
      --input "${RUST_SUMMARY_CSV}" \
      --output-cms "${PLOTS_DIR}/${RESULT_PREFIX}_cms.png" \
      --output-cs "${PLOTS_DIR}/${RESULT_PREFIX}_cs.png" \
      --output-hll "${PLOTS_DIR}/${RESULT_PREFIX}_hll.png"

    echo "Wrote ${RUST_SUMMARY_CSV}"
    echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_cms.png"
    echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_cs.png"
    echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_hll.png"
  else
    "${SCRIPT_DIR}/run_throughput_rust.sh" "${VARIANT}" "${RUST_SUMMARY_CSV}"

    if [[ "${RUST_ONLY}" == "true" ]]; then
      cp "${RUST_SUMMARY_CSV}" "${SUMMARY_CSV_PATH}"
    else
      "${SCRIPT_DIR}/run_throughput_cpp.sh" "${VARIANT}" "${CPP_SUMMARY_CSV}"
      {
        echo "${SUMMARY_HEADER}"
        tail -n +2 "${RUST_SUMMARY_CSV}"
        tail -n +2 "${CPP_SUMMARY_CSV}"
      } > "${SUMMARY_CSV_PATH}"
    fi

    case "${VARIANT}" in
      cms)
        python3 "${SCRIPT_DIR}/plot_cms_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
      cs)
        python3 "${SCRIPT_DIR}/plot_cs_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
      hll)
        python3 "${SCRIPT_DIR}/plot_hll_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
      kll)
        python3 "${SCRIPT_DIR}/plot_kll_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
      cms32k)
        python3 "${SCRIPT_DIR}/plot_cms32k_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
      cs32k)
        python3 "${SCRIPT_DIR}/plot_cs32k_throughput.py" \
          --input "${SUMMARY_CSV_PATH}" \
          --output "${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
        ;;
    esac

    echo "Wrote ${SUMMARY_CSV_PATH}"
    echo "Wrote ${PLOTS_DIR}/${RESULT_PREFIX}_insertion.png"
  fi
}

REQUESTED="${1:-${THROUGHPUT_VARIANT:-all}}"

case "${REQUESTED}" in
  cms|cs|hll|kll|octo|cms32k|cs32k)
    run_variant "${REQUESTED}"
    ;;
  all)
    for v in cms cs hll kll octo cms32k cs32k; do
      run_variant "$v"
    done
    ;;
  *)
    echo "unsupported variant: ${REQUESTED}; expected cms, cs, hll, kll, octo, cms32k, cs32k, or all" >&2
    exit 1
    ;;
esac
