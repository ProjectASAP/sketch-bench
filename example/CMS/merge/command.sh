#!/usr/bin/env bash
# CMS: folding eight shards into one, read as a time and as a rate
#
# One request, written two ways. The only difference between the two commands
# below is `--flat`.
set -euo pipefail
BIN=${BIN:-../../../target/release/approxbench}

# One JSONL record per square measured.
"$BIN" sketchbench \
    --algorithm cms-fastpath-vector2d \
    --impl lib \
    --config 'rows=4 cols=32768' \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations merge \
    --metrics latency,throughput \
    --merge-shards 8 \
    > records.jsonl

# The same squares folded into one row: one slot per operation, one field per
# metric.
"$BIN" sketchbench \
    --algorithm cms-fastpath-vector2d \
    --impl lib \
    --config 'rows=4 cols=32768' \
    --workload zipf \
    --size 200000 \
    --cardinality 20000 \
    --zipf-s 1.1 \
    --runs 5 \
    --warmup-runs 2 \
    --operations merge \
    --metrics latency,throughput \
    --merge-shards 8 \
    --flat \
    > flat.json
