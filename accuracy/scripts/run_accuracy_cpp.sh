#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACCURACY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VARIANT="${3:-${ACCURACY_VARIANT:-cms}}"

# Map variant → (crate root dir, cpp source subdir). The cpp trees
# live under the statistic-based crates now:
#   accuracy/cardinality/cpp/         — HLL
#   accuracy/frequency/cpp/{cms,countsketch}/
#   accuracy/kll/cpp/                  — KLL
CRATE_DIR=""
CPP_SUBDIR=""
case "${VARIANT}" in
  hll|cardinality)
    CRATE_DIR="${ACCURACY_DIR}/cardinality"
    CPP_SUBDIR=""
    ;;
  cms)
    CRATE_DIR="${ACCURACY_DIR}/frequency"
    CPP_SUBDIR="cms"
    ;;
  cs|countsketch|dd|nitro|octo)
    echo "${VARIANT} variant has no C++ accuracy baseline; skipping C++ build."
    exit 0
    ;;
  kll)
    CRATE_DIR="${ACCURACY_DIR}/kll"
    CPP_SUBDIR=""
    ;;
  *)
    echo "unsupported variant: ${VARIANT}; expected cardinality (hll), cms, kll, cs, dd, nitro, or octo" >&2
    exit 1
    ;;
esac

RESULT_PREFIX="${VARIANT}_accuracy"
OUTPUT_DIR="${CRATE_DIR}/output"
DATA_DST="${4:-${ACCURACY_DATASET:-${ACCURACY_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin}}"

if [[ -n "${CPP_SUBDIR}" ]]; then
  CPP_SRC_DIR="${CRATE_DIR}/cpp/${CPP_SUBDIR}"
  BUILD_DIR="${CRATE_DIR}/cpp/${CPP_SUBDIR}/build"
else
  CPP_SRC_DIR="${CRATE_DIR}/cpp"
  BUILD_DIR="${CRATE_DIR}/cpp/build"
fi

SUMMARY_OUTPUT_PATH="${1:-${OUTPUT_DIR}/${RESULT_PREFIX}_results_cpp.csv}"
KEY_SEED_ERROR_OUTPUT_PATH="${ACCURACY_KEY_SEED_ERRORS_OUTPUT_PATH:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_seed_errors_cpp.csv}"

mkdir -p "${ACCURACY_DIR}/../input" "${OUTPUT_DIR}" "${BUILD_DIR}"

# The accuracy tree was split into per-variant source roots, so an older cache
# may still point at the previous top-level source directory.
rm -f "${BUILD_DIR}/CMakeCache.txt"
rm -rf "${BUILD_DIR}/CMakeFiles"

if [[ ! -f "${DATA_DST}" ]]; then
  if [[ "${DATA_DST}" == "${ACCURACY_DIR}/../input/benchmark_data_10m_int64_zipf_s11_k100000.bin" ]]; then
    "${ACCURACY_DIR}/../input/generate_zipf_data.sh"
  else
    echo "dataset not found: ${DATA_DST}" >&2
    exit 1
  fi
fi

cmake -S "${CPP_SRC_DIR}" -B "${BUILD_DIR}" -DCMAKE_BUILD_TYPE=Release
cmake --build "${BUILD_DIR}" --config Release

if [[ "${VARIANT}" == "cardinality" || "${VARIANT}" == "hll" ]]; then
  "${BUILD_DIR}/hll_accuracy" --data "${DATA_DST}" > "${SUMMARY_OUTPUT_PATH}"
elif [[ "${VARIANT}" == "kll" ]]; then
  "${BUILD_DIR}/kll_accuracy" --data "${DATA_DST}" > "${SUMMARY_OUTPUT_PATH}"
else
  # cms
  KEY_ERROR_OUTPUT_PATH="${2:-${OUTPUT_DIR}/${RESULT_PREFIX}_key_median_errors_cpp.csv}"
  "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode summary > "${SUMMARY_OUTPUT_PATH}"
  "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode key-errors > "${KEY_ERROR_OUTPUT_PATH}"
  "${BUILD_DIR}/cms_accuracy" --data "${DATA_DST}" --mode key-seed-errors > "${KEY_SEED_ERROR_OUTPUT_PATH}"
fi
