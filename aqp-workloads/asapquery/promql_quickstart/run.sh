#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Run the ASAPQuery PromQL quickstart benchmark and import it into AQP JSONL.

Usage:
  aqp-workloads/asapquery/promql_quickstart/run.sh [options]

Options:
  --asapquery-dir PATH   ASAPQuery checkout. Default: ../ASAPQuery from sketch-bench.
  --output PATH          AQP JSONL output. Default: this bundle's latest_report.jsonl.
  --python PATH          Python interpreter for ASAPQuery benchmark scripts.
  --runs N               Paired timestamp runs per query. Default: 3.
  --diagnostics-output PATH
                          Paired timestamp diagnostics JSON output.
                          Default: ASAPQuery/benchmarks/reports/paired_diagnostics.json.
  --legacy-asapquery-scripts
                          Use ASAPQuery's sequential run_baseline.py and run_asap.py
                          instead of the paired timestamp runner.
  --keep-stack           Leave Docker quickstart services running after the run.
  -h, --help             Show this help.

Environment:
  DOCKER_DEFAULT_PLATFORM defaults to linux/amd64 because ASAPQuery's published
  quickstart images may not have arm64 manifests.
USAGE
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKETCH_BENCH_DIR="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
ASAPQUERY_DIR="${ASAPQUERY_DIR:-"${SKETCH_BENCH_DIR}/../ASAPQuery"}"
OUTPUT="${SCRIPT_DIR}/latest_report.jsonl"
PYTHON_BIN="${PYTHON:-python3}"
KEEP_STACK=0
RUNS_PER_QUERY=3
LEGACY_ASAPQUERY_SCRIPTS=0
DIAGNOSTICS_OUTPUT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --asapquery-dir)
      ASAPQUERY_DIR="$2"
      shift 2
      ;;
    --output)
      OUTPUT="$2"
      shift 2
      ;;
    --python)
      PYTHON_BIN="$2"
      shift 2
      ;;
    --runs)
      RUNS_PER_QUERY="$2"
      shift 2
      ;;
    --diagnostics-output)
      DIAGNOSTICS_OUTPUT="$2"
      shift 2
      ;;
    --legacy-asapquery-scripts)
      LEGACY_ASAPQUERY_SCRIPTS=1
      shift
      ;;
    --keep-stack)
      KEEP_STACK=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

ASAPQUERY_DIR="$(cd "${ASAPQUERY_DIR}" && pwd)"
QUICKSTART_DIR="${ASAPQUERY_DIR}/asap-quickstart"
BENCHMARK_DIR="${ASAPQUERY_DIR}/benchmarks"
REPORT_DIR="${BENCHMARK_DIR}/reports"
MANIFEST="${SCRIPT_DIR}/manifest.toml"
PAIRED_RUNNER="${SCRIPT_DIR}/paired_promql_runner.py"
QUERY_SUITE="${BENCHMARK_DIR}/queries/promql_suite.json"
BASELINE_JSON="${REPORT_DIR}/baseline_results.json"
ASAP_JSON="${REPORT_DIR}/asap_results.json"
EVAL_REPORT="${REPORT_DIR}/eval_report.md"
if [[ -z "${DIAGNOSTICS_OUTPUT}" ]]; then
  DIAGNOSTICS_OUTPUT="${REPORT_DIR}/paired_diagnostics.json"
fi
DOCKER_PLATFORM="${DOCKER_DEFAULT_PLATFORM:-linux/amd64}"
TEMP_VENV=""

cleanup() {
  local exit_code=$?
  set +e
  if [[ "${KEEP_STACK}" -eq 0 ]]; then
    echo "[aqp-asapquery-promql] stopping quickstart stack"
    (cd "${QUICKSTART_DIR}" && DOCKER_DEFAULT_PLATFORM="${DOCKER_PLATFORM}" docker compose -f docker-compose.yml down)
  else
    echo "[aqp-asapquery-promql] keeping quickstart stack running"
  fi
  if [[ -n "${TEMP_VENV}" ]]; then
    rm -rf "${TEMP_VENV}"
  fi
  exit "${exit_code}"
}
trap cleanup EXIT

require_path() {
  local path="$1"
  local description="$2"
  if [[ ! -e "${path}" ]]; then
    echo "missing ${description}: ${path}" >&2
    exit 1
  fi
}

ensure_python_requests() {
  if "${PYTHON_BIN}" -c 'import requests' >/dev/null 2>&1; then
    echo "${PYTHON_BIN}"
    return
  fi

  TEMP_VENV="$(mktemp -d "${TMPDIR:-/tmp}/aqp-asapquery-promql-venv.XXXXXX")"
  "${PYTHON_BIN}" -m venv "${TEMP_VENV}"
  "${TEMP_VENV}/bin/python" -m pip install requests >&2
  echo "${TEMP_VENV}/bin/python"
}

require_path "${QUICKSTART_DIR}/docker-compose.yml" "ASAPQuery quickstart compose file"
require_path "${BENCHMARK_DIR}/scripts/wait_for_stack.sh" "ASAPQuery wait script"
require_path "${BENCHMARK_DIR}/scripts/ingest_wait.sh" "ASAPQuery ingest wait script"
require_path "${BENCHMARK_DIR}/scripts/run_baseline.py" "ASAPQuery baseline script"
require_path "${BENCHMARK_DIR}/scripts/run_asap.py" "ASAPQuery ASAP script"
require_path "${BENCHMARK_DIR}/scripts/compare.py" "ASAPQuery compare script"
require_path "${QUERY_SUITE}" "ASAPQuery PromQL query suite"
require_path "${MANIFEST}" "AQP manifest"
require_path "${PAIRED_RUNNER}" "paired PromQL runner"

mkdir -p "${REPORT_DIR}"
mkdir -p "$(dirname "${OUTPUT}")"
RUN_PYTHON="$(ensure_python_requests)"

SERVICES=(
  prometheus
  asap-planner-rs
  queryengine
  fake-exporter-constant
  fake-exporter-linear-up
  fake-exporter-linear-down
  fake-exporter-sine
  fake-exporter-sine-noise
  fake-exporter-step
  fake-exporter-exp-up
)

echo "[aqp-asapquery-promql] starting quickstart stack from ${QUICKSTART_DIR}"
(cd "${QUICKSTART_DIR}" && DOCKER_DEFAULT_PLATFORM="${DOCKER_PLATFORM}" docker compose -f docker-compose.yml up -d "${SERVICES[@]}")

echo "[aqp-asapquery-promql] waiting for Prometheus and QueryEngine"
(cd "${ASAPQUERY_DIR}" && bash benchmarks/scripts/wait_for_stack.sh)

echo "[aqp-asapquery-promql] waiting for sketches to accumulate"
(cd "${ASAPQUERY_DIR}" && bash benchmarks/scripts/ingest_wait.sh)

if [[ "${LEGACY_ASAPQUERY_SCRIPTS}" -eq 1 ]]; then
  echo "[aqp-asapquery-promql] running legacy ASAPQuery Prometheus baseline"
  (cd "${ASAPQUERY_DIR}" && "${RUN_PYTHON}" benchmarks/scripts/run_baseline.py --output "${BASELINE_JSON}")

  echo "[aqp-asapquery-promql] running legacy ASAPQuery endpoint"
  (cd "${ASAPQUERY_DIR}" && "${RUN_PYTHON}" benchmarks/scripts/run_asap.py --output "${ASAP_JSON}")
else
  echo "[aqp-asapquery-promql] running paired timestamp PromQL benchmark"
  "${RUN_PYTHON}" "${PAIRED_RUNNER}" \
    --query-suite "${QUERY_SUITE}" \
    --baseline-output "${BASELINE_JSON}" \
    --asap-output "${ASAP_JSON}" \
    --diagnostics-output "${DIAGNOSTICS_OUTPUT}" \
    --runs "${RUNS_PER_QUERY}"
fi

echo "[aqp-asapquery-promql] running ASAPQuery comparator"
(cd "${ASAPQUERY_DIR}" && "${RUN_PYTHON}" benchmarks/scripts/compare.py --baseline "${BASELINE_JSON}" --asap "${ASAP_JSON}" --output "${EVAL_REPORT}")

echo "[aqp-asapquery-promql] importing into AQP JSONL"
: > "${OUTPUT}"
(cd "${SKETCH_BENCH_DIR}" && cargo run -p sketch-cli --no-default-features -- aqp import-asapquery-promql \
  --manifest "${MANIFEST}" \
  --baseline-json "${BASELINE_JSON}" \
  --asap-json "${ASAP_JSON}" \
  --query-suite "${QUERY_SUITE}" \
  --report "${OUTPUT}")

echo "[aqp-asapquery-promql] wrote ${OUTPUT}"
