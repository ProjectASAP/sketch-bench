#!/usr/bin/env bash
# One command: fetch and build everything at the pins in pins.env, then
# compare ASAPQuery-backend against Prometheus on the p99 latency query.
#
#   e2e/run.sh [OUT_DIR] [extra aqpbm-e2e flags...]
#
# Results land in OUT_DIR (default e2e/results/run-<UTC timestamp>):
# queries.csv, answers.csv, summary.json, and per-arm plans and logs.
set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$E2E_DIR/.." && pwd)"
source "$E2E_DIR/pins.env"
CACHE_DIR="${E2E_CACHE_DIR:-$E2E_DIR/.cache}"
TARGET="${CARGO_TARGET_DIR:-$REPO_DIR/target}"
OUT_DIR="${1:-$E2E_DIR/results/run-$(date -u +%Y%m%dT%H%M%SZ)}"
shift || true

"$E2E_DIR/setup.sh"

# Two arms over the same samples and evaluation times:
#   query_time_raw   default capabilities: raw data stays in Prometheus, the
#                    plan builds a KLL sketch from a 60m raw window read from
#                    Prometheus at query time.
#   ingestion_panes  {"raw_data_retained": false}: the plan builds a KLL per
#                    10m pane at ingestion and merges six panes at query time.
"$TARGET/release/aqpbm-e2e" \
  --prometheus-bin "$CACHE_DIR/prometheus-${PROMETHEUS_VERSION}.linux-amd64/prometheus" \
  --control-plane-bin "$TARGET/release/control_plane" \
  --data-plane-bin "$TARGET/release/data_plane" \
  --workload "$E2E_DIR/workloads/latency_p99_60m.json" \
  --arm query_time_raw \
  --arm "ingestion_panes=$E2E_DIR/capabilities/no_raw.json" \
  --pins "$E2E_DIR/pins.env" \
  --out-dir "$OUT_DIR" \
  "$@"
