#!/usr/bin/env bash
# One command: fetch and build everything at the pins in pins.env, then
# compare ASAPQuery-backend against Prometheus on the p99 latency query.
#
#   aqpbm-planeval/run.sh [OUT_DIR] [extra planeval flags...]
#
# Results land in OUT_DIR (default aqpbm-planeval/results/run-<UTC timestamp>):
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

# Four arms over the same samples and evaluation times, one per deployment
# configuration the control plane plans with:
#   default                    default capabilities: raw data stays in
#                              Prometheus; only outputs with an accuracy
#                              guarantee are selected.
#   default_ignore_guarantees  the same, with --ignore-accuracy-guarantees.
#   no_raw                     {"raw_data_retained": false}: every input is
#                              built at ingestion time.
#   no_raw_ignore_guarantees   the same, with --ignore-accuracy-guarantees.
"$TARGET/release/planeval" \
  --prometheus-bin "$CACHE_DIR/prometheus-${PROMETHEUS_VERSION}.linux-amd64/prometheus" \
  --control-plane-bin "$TARGET/release/control_plane" \
  --data-plane-bin "$TARGET/release/data_plane" \
  --workload "$E2E_DIR/workloads/latency_p99_60m.json" \
  --arm default \
  --arm default_ignore_guarantees,ignore-accuracy-guarantees \
  --arm "no_raw,caps=$E2E_DIR/capabilities/no_raw.json" \
  --arm "no_raw_ignore_guarantees,caps=$E2E_DIR/capabilities/no_raw.json,ignore-accuracy-guarantees" \
  --pins "$E2E_DIR/pins.env" \
  --out-dir "$OUT_DIR" \
  "$@"
