#!/usr/bin/env bash
# KLL: scored against ground truth
#
# One request, written two ways. The only difference between the two commands
# below is `--flat`.
set -euo pipefail
BIN=${BIN:-../../../target/release/approxbench}

# One JSONL record per square measured.
"$BIN" sketchbench \
    --algorithm kll-cdf \
    --impl lib \
    --config k=200 \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations query \
    --metrics accuracy \
    --comparator rank-error \
    > records.jsonl

# The same squares folded into one row: one slot per operation, one field per
# metric.
"$BIN" sketchbench \
    --algorithm kll-cdf \
    --impl lib \
    --config k=200 \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations query \
    --metrics accuracy \
    --comparator rank-error \
    --flat \
    > flat.json
