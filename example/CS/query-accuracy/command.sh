#!/usr/bin/env bash
# CS: scored against ground truth
#
# One request, written two ways. The only difference between the two commands
# below is `--flat`.
set -euo pipefail
BIN=${BIN:-../../../target/release/approxbench}

# One JSONL record per square measured.
"$BIN" sketchbench \
    --algorithm countsketch-fastpath-vector2d \
    --impl lib \
    --config 'rows=4 cols=32768' \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations query \
    --metrics accuracy \
    --comparator frequency \
    > records.jsonl

# The same squares folded into one row: one slot per operation, one field per
# metric.
"$BIN" sketchbench \
    --algorithm countsketch-fastpath-vector2d \
    --impl lib \
    --config 'rows=4 cols=32768' \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations query \
    --metrics accuracy \
    --comparator frequency \
    --flat \
    > flat.json
