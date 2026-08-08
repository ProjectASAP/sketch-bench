#!/usr/bin/env bash
# HLL: insert and query, each timed two ways
#
# One request, written two ways. The only difference between the two commands
# below is `--flat`.
set -euo pipefail
BIN=${BIN:-../../../target/release/approxbench}

# One JSONL record per square measured.
"$BIN" sketchbench \
    --algorithm hll \
    --impl lib \
    --config lg_k=14 \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations insert,query \
    --metrics throughput,latency,cpu,memory \
    --comparator cardinality \
    > records.jsonl

# The same squares folded into one row: one slot per operation, one field per
# metric.
"$BIN" sketchbench \
    --algorithm hll \
    --impl lib \
    --config lg_k=14 \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations insert,query \
    --metrics throughput,latency,cpu,memory \
    --comparator cardinality \
    --flat \
    > flat.json
