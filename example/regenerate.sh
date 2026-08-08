#!/usr/bin/env bash
# Regenerate every example under this directory.
#
# Each leaf holds one measurement request written twice: `command.sh` runs it
# with and without `--flat`, and the two output files are what those two runs
# printed. Nothing here is typed by hand, so a change in the record shape shows
# up as a diff instead of as prose that quietly stopped being true.
#
#   ./example/regenerate.sh
#
# Built in release, because a debug build measures a different machine.
set -euo pipefail

cd "$(dirname "$0")/.."
cargo build --release --bin approxbench >&2
BIN="$PWD/target/release/approxbench"
EXAMPLE="$PWD/example"

# One workload for all four sketches, so numbers in different directories are
# taken over the same stream and can be read side by side.
WORKLOAD=(--workload zipf --size 200000 --cardinality 20000 --zipf-s 1.1)
REPETITION=(--runs 5 --warmup-runs 2)

# Render one argument as a reader can paste it: quoted only where it has to be.
quoted() {
    case "$1" in
        *[[:space:]]*) printf "'%s'" "$1" ;;
        *)             printf '%s' "$1" ;;
    esac
}

# leaf <dir> <title> <request...>
#
# The request carries `--operations` and `--metrics`, which is the point of the
# example: they are what names the squares a run measures.
leaf() {
    local dir="$1" title="$2"; shift 2
    local request=("$@")
    local out="$EXAMPLE/$dir"
    mkdir -p "$out"

    # The argument list for pasting into command.sh, a flag and its value on one
    # line so a reader sees the pairs and not a column of loose words.
    local rendered="" line="" arg
    for arg in "${request[@]}"; do
        if [[ "$arg" == --* ]]; then
            [[ -n "$line" ]] && rendered+="    $line \\"$'\n'
            line="$arg"
        else
            line+=" $(quoted "$arg")"
        fi
    done
    [[ -n "$line" ]] && rendered+="    $line \\"$'\n'

    cat > "$out/command.sh" <<EOF
#!/usr/bin/env bash
# $title
#
# One request, written two ways. The only difference between the two commands
# below is \`--flat\`.
set -euo pipefail
BIN=\${BIN:-../../../target/release/approxbench}

# One JSONL record per square measured.
"\$BIN" sketchbench \\
${rendered}    > records.jsonl

# The same squares folded into one row: one slot per operation, one field per
# metric.
"\$BIN" sketchbench \\
${rendered}    --flat \\
    > flat.json
EOF
    chmod +x "$out/command.sh"

    "$BIN" sketchbench "${request[@]}" 2>/dev/null > "$out/records.jsonl"
    "$BIN" sketchbench "${request[@]}" --flat 2>/dev/null > "$out/flat.json"

    printf '  %-26s %s record(s) -> 1 row\n' "$dir" "$(wc -l < "$out/records.jsonl")" >&2
}

# sketch <dir> <algorithm> <config> <comparator>
#
# Three leaves each, one per shape of request: two operations sharing their
# metrics, one operation scored against the truth, and one operation read for
# two metrics. Every square runs its own passes over its own runs, so two
# squares of one operation are two measurements and not one measurement twice.
sketch() {
    local dir="$1" algorithm="$2" config="$3" comparator="$4"
    local row=(--algorithm "$algorithm" --impl lib --config "$config")

    leaf "$dir/insert-and-query" \
        "$dir: insert and query, each timed two ways" \
        "${row[@]}" "${WORKLOAD[@]}" "${REPETITION[@]}" \
        --operations insert,query \
        --metrics throughput,latency,cpu,memory \
        --comparator "$comparator"

    leaf "$dir/query-accuracy" \
        "$dir: scored against ground truth" \
        "${row[@]}" "${WORKLOAD[@]}" "${REPETITION[@]}" \
        --operations query --metrics accuracy \
        --comparator "$comparator"

    leaf "$dir/merge" \
        "$dir: folding eight shards into one, read as a time and as a rate" \
        "${row[@]}" "${WORKLOAD[@]}" "${REPETITION[@]}" \
        --operations merge --metrics latency,throughput \
        --merge-shards 8
}

echo "regenerating examples" >&2

# CMS and CountSketch answer frequency. asap_sketchlib exposes neither under a
# bare algorithm name: its counting sketches are structural variants, so the
# storage path is part of what the row is. Both `vector2d` rows here, because
# the `fixedmatrix` ones admit no comparator and so cannot fill the accuracy
# square.
sketch CMS cms-fastpath-vector2d         'rows=4 cols=32768' frequency
sketch CS  countsketch-fastpath-vector2d 'rows=4 cols=32768' frequency

# HLL and KLL do have bare rows in this library.
sketch HLL hll     'lg_k=14' cardinality
sketch KLL kll-cdf 'k=200'   rank-error

echo "done" >&2
