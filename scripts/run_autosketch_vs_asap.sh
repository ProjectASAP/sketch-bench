#!/usr/bin/env bash
# Run the AutoSketch vs. ASAP evaluation on the `traces` workloads
# (ProjectASAP/ASAPQuery#777) and plot it.
#
# Usage: scripts/run_autosketch_vs_asap.sh
#
# `traces` reads rqe-optimizer/data/autosketch-eval/table.json.

set -euo pipefail

OUT=rqe-optimizer/results/autosketch-vs-asap
BIN=./target/release/examples/autosketch_vs_asap
mkdir -p "$OUT"

cargo build --release -p rqe-optimizer --example autosketch_vs_asap

for dataset in alibaba_v2022 google_2011 boom; do
    "$BIN" traces --dataset "$dataset" --out "$OUT/traces-$dataset.json" --runs 5
done

python3 scripts/plot_autosketch_vs_asap.py "$OUT"
