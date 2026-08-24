#!/usr/bin/env bash
# show_atomic_cost_trends.sh — dump out/atomic_costs.json as one sorted,
# column-aligned table per sketch algorithm so cost-vs-param trends are visible
# at a glance. Usage: scripts/show_atomic_cost_trends.sh [path/to/atomic_costs.json]

set -euo pipefail
FILE="${1:-out/atomic_costs.json}"

for sketch in $(jq -r '[.[].sketch] | unique[]' "$FILE"); do
    echo "== $sketch =="
    jq -r --arg s "$sketch" '
        .[] | select(.sketch == $s) |
        [
            (.sketch_config.params | to_entries | map("\(.key)=\(.value)") | join(",")),
            .mem_bytes_per_instance,
            .insert_cpu_secs,
            .query_cpu_secs,
            .merge_cpu_secs,
            (.query_accuracy | to_entries | map("\(.key)=\(.value)") | join(","))
        ] | @tsv
    ' "$FILE" | sort -t= -k2 -n | \
        (echo -e "params\tmem_bytes\tinsert_cpu_secs\tquery_cpu_secs\tmerge_cpu_secs\tquery_accuracy"; cat) | \
        column -t -s $'\t'
    echo
done
