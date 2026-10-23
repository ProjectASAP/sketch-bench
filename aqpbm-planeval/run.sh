#!/usr/bin/env bash
# One command: fetch and build everything at the pins in pins.env, then
# compare ASAPQuery-backend against Prometheus on a workload of template
# queries (PLANEVAL_WORKLOAD, default workloads/templates.json).
#
#   aqpbm-planeval/run.sh [OUT_DIR] [extra planeval flags...]
#
# Results land in OUT_DIR (default aqpbm-planeval/results/run-<UTC timestamp>):
# per_query.csv, probes.csv, queries.csv, answers.csv, load.csv,
# summary.json, and per arm the probe plans and each joint batch's plan and logs.
set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$E2E_DIR/.." && pwd)"
source "$E2E_DIR/pins.env"
CACHE_DIR="${E2E_CACHE_DIR:-$E2E_DIR/.cache}"
TARGET="${CARGO_TARGET_DIR:-$REPO_DIR/target}"
OUT_DIR="${1:-$E2E_DIR/results/run-$(date -u +%Y%m%dT%H%M%SZ)}"
shift || true

"$E2E_DIR/setup.sh"

# Series sit on the label hierarchy PLANEVAL_LABELS (default
# region:2,service:4,instance:2, 16 series); --series N rescales its last
# level. A counter, requests_total, is written next to the gauge latency_ms.
# PLANEVAL_SCALE (default service:1,10) gives every service its own factor on
# both, so a top-k over services ranks real differences; empty for none.
#
# Each arm plans every query alone first, then the supported ones jointly in
# batches of --joint-size (default 6). Four arms over the same samples and
# evaluation times, one per deployment configuration the control plane plans
# with:
#   default                    default capabilities: raw data stays in
#                              Prometheus; only outputs with an accuracy
#                              guarantee are selected.
#   default_ignore_guarantees  the same, with --ignore-accuracy-guarantees.
#   no_raw                     {"raw_data_retained": false}: every input is
#                              built at ingestion time.
#   no_raw_ignore_guarantees   the same, with --ignore-accuracy-guarantees.
SCALE="${PLANEVAL_SCALE-service:1,10}"
SCALE_ARGS=()
[[ -n "$SCALE" ]] && SCALE_ARGS=(--scale "$SCALE")
"$TARGET/release/planeval" \
  --prometheus-bin "$CACHE_DIR/prometheus-${PROMETHEUS_VERSION}.linux-amd64/prometheus" \
  --control-plane-bin "$TARGET/release/control_plane" \
  --data-plane-bin "$TARGET/release/data_plane" \
  --workload "${PLANEVAL_WORKLOAD:-$E2E_DIR/workloads/templates.json}" \
  --arm default \
  --arm default_ignore_guarantees,ignore-accuracy-guarantees \
  --arm "no_raw,caps=$E2E_DIR/capabilities/no_raw.json" \
  --arm "no_raw_ignore_guarantees,caps=$E2E_DIR/capabilities/no_raw.json,ignore-accuracy-guarantees" \
  --labels "${PLANEVAL_LABELS:-region:2,service:4,instance:2}" \
  "${SCALE_ARGS[@]}" \
  --pins "$E2E_DIR/pins.env" \
  --out-dir "$OUT_DIR" \
  "$@"
