#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out_dir=${1:-"$repo_dir/results/erp-v1-figure1"}
raw="$out_dir/raw.jsonl"
if [[ -e "$raw" ]]; then
  echo "refusing to overwrite $raw" >&2
  exit 2
fi
mkdir -p "$out_dir"

export BENCH_WARMUP_SECS=${BENCH_WARMUP_SECS:-1}
bin="$repo_dir/target/release/approxbench"

run_point() {
  local distribution=$1 rows=$2 cols=$3 burst=$4
  local dataset=(--dataset "$distribution" --cardinality 10000 --size 1000000)
  [[ "$distribution" == zipf ]] && dataset+=(--zipf-s 1.1)
  local burst_args=()
  if [[ "$burst" == burst ]]; then
    burst_args+=(--burst-interval-rows 100000 --burst-intervals 2 --burst-extra-fraction 0.5)
  fi
  "$bin" sketchbench --variant cms --library oxide \
    --config "rows=$rows cols=$cols" "${dataset[@]}" --dtype i64 --seed 42 \
    "${burst_args[@]}" --runs 7 --warmup-runs 2 \
    --operations insert,merge,query --metrics throughput,cpu,memory --report "$raw"
  "$bin" sketchbench --variant cms --library oxide \
    --config "rows=$rows cols=$cols" "${dataset[@]}" --dtype i64 --seed 42 \
    "${burst_args[@]}" --runs 1 --warmup-runs 1 \
    --operations query --metrics accuracy --report "$raw"
}

for distribution in uniform zipf; do
  for rows in 3 5; do
    for cols in 256 1024 4096; do
      run_point "$distribution" "$rows" "$cols" steady
    done
  done
done
for rows in 3 5; do
  for cols in 256 1024 4096; do
    run_point zipf "$rows" "$cols" burst
  done
done

python3 "$repo_dir/scripts/analyze_erp_v1_figure1.py" "$raw" "$out_dir"
