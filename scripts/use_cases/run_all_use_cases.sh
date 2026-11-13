#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"

usage() {
  cat >&2 <<EOF
usage: $0 [--use-case 1,2,3,4,5] [--quick] [--no-grafana]

Builds planeval, runs each use case on its synthetic workload one after another, writes the rows to
target/grafana-data, then serves the dashboards at http://localhost:3000.

  --use-case LIST  comma-separated use cases to run; default all
  --quick          run one approximate arm per use case instead of every arm
  --no-grafana     write the rows and stop

On an 8-core laptop the build takes about 11 minutes; then --quick takes about 11 minutes and every arm
about 1 hour 40 minutes, most of it in the 0.01 and tighter budgets of use cases 1 and 2. Peak memory
stays under 4 GB, since the use cases run one at a time.

Needs cargo, python3 and Grafana (brew install grafana, or GRAFANA_HOME set to an unpacked release).
Grafana's state and the Infinity plugin live under target/grafana-home.
EOF
  exit 2
}

cases=$(python3 -c "import sys; sys.path.insert(0, 'scripts/use_cases'); import cases; print(','.join(cases.load()))")
quick=0
grafana=1
while [ $# -gt 0 ]; do
  case $1 in
    --use-case) [ $# -ge 2 ] || usage; cases=$2; shift 2 ;;
    --quick) quick=1; shift ;;
    --no-grafana) grafana=0; shift ;;
    *) usage ;;
  esac
done

command -v cargo >/dev/null || { echo "cargo not found; install Rust from https://rustup.rs" >&2; exit 1; }
if [ $grafana = 1 ]; then
  tools/grafana/run.sh --check
fi

cargo build --release -p aqpbm-planeval

for case_id in ${cases//,/ }; do
  arm=()
  if [ $quick = 1 ]; then
    arm=(--arm "$(python3 -c "import sys; sys.path.insert(0, 'scripts/use_cases'); import cases
print(next(a for a in cases.load()['$case_id'].CASE['arms'] if a.startswith('noraw')))")")
  fi
  echo "== use case $case_id ${arm[*]:-every arm}" >&2
  started=$SECONDS
  python3 scripts/use_cases/run_use_case.py "$case_id" ${arm[@]+"${arm[@]}"}
  echo "== use case $case_id took $((SECONDS - started)) s" >&2
done

if [ $grafana = 1 ]; then
  echo "the phase3-* dashboards stay empty until scripts/use_cases/run_use_case_on_systems.py has run" >&2
  exec tools/grafana/run.sh
fi
