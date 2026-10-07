#!/usr/bin/env bash
# Run the AutoSketch vs. ASAP evaluation on the `traces` workloads
# (ProjectASAP/ASAPQuery#777) and plot it.
#
# Usage: scripts/run_autosketch_vs_asap.sh SATURATION_DIR
#
# `traces` reads rqe-optimizer/data/autosketch-eval/table.json for the
# workload. SATURATION_DIR holds the saturation curves (out_grid_1e7_cost/,
# out_1e9/) and the cost table (optimizer_cost/) from
# scripts/study_saturation.py, the same inputs the planner reads.

set -euo pipefail

SATURATION_DIR=${1:?usage: $0 SATURATION_DIR}

OUT=rqe-optimizer/results/autosketch-vs-asap
BIN=./target/release/examples/autosketch_vs_asap
mkdir -p "$OUT"

cargo build --release -p rqe-optimizer --example autosketch_vs_asap

for dataset in alibaba_v2022 google_2011 boom; do
    "$BIN" traces --dataset "$dataset" --saturation-dir "$SATURATION_DIR" \
        --out "$OUT/traces-$dataset.json" --runs 5
done

python3 scripts/plot_autosketch_vs_asap.py "$OUT"
