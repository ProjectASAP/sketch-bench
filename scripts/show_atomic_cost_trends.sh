#!/usr/bin/env bash
# show_atomic_cost_trends.sh — dump out/atomic_costs.json (an
# AtomicCostDocument: {schema_version, profiles: [{workload, entries}]}) as
# one sorted, column-aligned table per (workload profile, sketch algorithm)
# so cost-vs-param trends are visible at a glance. Usage:
# scripts/show_atomic_cost_trends.sh [path/to/atomic_costs.json]

set -euo pipefail
FILE="${1:-out/atomic_costs.json}"

# One label per profile: distinguishes profiles when several workloads (or
# several external windows over the same dataset) are in one document, so a
# reader can't mistake one profile's rows for another's.
workload_label() {
    jq -r '
        if .workload.synthetic then "synthetic"
        else "external:\(.workload.external.source)/\(.workload.external.dataset) " +
             "window=[\(.workload.external.window_start_ns),\(.workload.external.window_end_ns))"
        end
    '
}

profile_count=$(jq '.profiles | length' "$FILE")
for p in $(seq 0 $((profile_count - 1))); do
    profile=$(jq -c --argjson p "$p" '.profiles[$p]' "$FILE")
    label=$(echo "$profile" | workload_label)
    echo "### workload: $label"

    for sketch in $(echo "$profile" | jq -r '[.entries[].sketch] | unique[]'); do
        echo "== $sketch =="
        echo "$profile" | jq -r --arg s "$sketch" '
            .entries[] | select(.sketch == $s) |
            [
                (.sketch_config.params | to_entries | map("\(.key)=\(.value)") | join(",")),
                .mem_bytes_per_instance,
                .insert_cpu_secs,
                .query_cpu_secs,
                .merge_cpu_secs,
                (.query_accuracy | to_entries | map("\(.key)=\(.value)") | join(","))
            ] | @tsv
        ' | sort -t= -k2 -n | \
            (echo -e "params\tmem_bytes\tinsert_cpu_secs\tquery_cpu_secs\tmerge_cpu_secs\tquery_accuracy"; cat) | \
            column -t -s $'\t'
        echo
    done
done
