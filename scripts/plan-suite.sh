#!/usr/bin/env bash
# Prepare a bench-external run directory from a suite: copy each workload and
# plan it once per variant with ASAPPlanner's `asap_planner --plan-file`.
#
#   scripts/plan-suite.sh <suite-dir> <run-dir>
#
# Environment: ASAP_PLANNER, the `asap_planner` binary (needs --plan-file
# and --executor-check, and deployment exclusions in capability files).
#
# A suite directory holds, per workload <w>: <w>.workload.json (the harness
# WorkloadSpec: queries q1..qN in planning order, and the data) and
# <w>.planner.json (the ASAPPlanner PlanningWorkload of the same queries).
#
# Variants (plans/<variant>/):
#   clickhouse.equivalent-only   capabilities/clickhouse.equivalent-only.json, executor check skipped
#   clickhouse.with-substitutes  capabilities/clickhouse.with-substitutes.json, executor check skipped
#   asap.default                 the planner's defaults, compiled for the Planner's executor
#   asap.exact                   every query's accuracy target exact (max_error 0, failure_probability 0), compiled
# A workload the planner selects no plan for gets <w>.no-plan.json with the
# planner's error line (or its last line) as the reason.
#
# Example: ASAP_PLANNER=target/release/asap_planner scripts/plan-suite.sh suites/showcase-1m /data/run-1
set -euo pipefail

suite=${1:?suite dir}; run=${2:?run dir}
: "${ASAP_PLANNER:?set ASAP_PLANNER to the asap_planner binary}"
repo=$(cd "$(dirname "$0")/.." && pwd)
caps="$repo/aqpbm-engines-external/capabilities"

[ -e "$run" ] && { echo "$run exists; runs are never overwritten" >&2; exit 1; }
compgen -G "$suite/*.workload.json" >/dev/null || { echo "$suite has no *.workload.json" >&2; exit 1; }
# planner-logs/ holds each planner run's log and the rewritten exact inputs.
mkdir -p "$run"/{workloads,data,plans,results,planner-logs}

variants=(
  "clickhouse.equivalent-only|$caps/clickhouse.equivalent-only.json|skip|false|keep"
  "clickhouse.with-substitutes|$caps/clickhouse.with-substitutes.json|skip|true|keep"
  "asap.default||compile|true|keep"
  "asap.exact||compile|false|exact"
)

for variant_line in "${variants[@]}"; do
  IFS='|' read -r variant _ _ substitutions _ <<<"$variant_line"
  mkdir -p "$run/plans/$variant"
  printf '{"substitutions_allowed": %s}\n' "$substitutions" > "$run/plans/$variant/variant.json"
done

for spec in "$suite"/*.workload.json; do
  w=$(basename "$spec" .workload.json)
  cp "$spec" "$run/workloads/$w.json"
  for variant_line in "${variants[@]}"; do
    IFS='|' read -r variant cap check substitutions accuracy <<<"$variant_line"
    planner_input="$suite/$w.planner.json"
    if [ "$accuracy" = exact ]; then
      planner_input="$run/planner-logs/$w.exact.planner.json"
      python3 - "$suite/$w.planner.json" "$planner_input" <<'PY'
import json, sys
workload = json.load(open(sys.argv[1]))
for query in workload["query_workload"]["repeating_queries"]:
    query.setdefault("requirements", {})["accuracy"] = {"max_error": 0.0, "failure_probability": 0.0}
json.dump(workload, open(sys.argv[2], "w"), indent=1)
PY
    fi
    args=(--workload "$planner_input" --run-until stage3 --executor-check "$check"
          --plan-file "$run/plans/$variant/$w.plan.json")
    [ -n "$cap" ] && args+=(--deployment-capabilities "$cap")
    log="$run/planner-logs/$w.$variant.log"
    if "$ASAP_PLANNER" "${args[@]}" > "$log" 2>&1 && [ -s "$run/plans/$variant/$w.plan.json" ]; then
      echo "$w $variant: plan"
    else
      # The planner's error line when there is one, else its last line.
      reason=$(grep -iE '^error[: ]' "$log" | tail -n 1 || true)
      [ -n "$reason" ] || reason=$(grep -v '^[[:space:]]*$' "$log" | tail -n 1 || true)
      python3 -c 'import json,sys; print(json.dumps({"reason": sys.argv[1]}))' "${reason:-no plan file written}" \
        > "$run/plans/$variant/$w.no-plan.json"
      rm -f "$run/plans/$variant/$w.plan.json"
      echo "$w $variant: no plan (${reason:-no plan file written})"
    fi
  done
done
