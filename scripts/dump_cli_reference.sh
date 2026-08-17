#!/usr/bin/env bash
# Compare the built binary's --help against docs/aqpbm-cli-reference.md.
#
# aqpbm-cli-reference.md is hand-authored: it states the intended command
# surface, and the code is changed to match it. This script dumps what clap
# actually produces so the gap is visible. It never overwrites the target.
#
#   ./scripts/dump_cli_reference.sh          diff the binary against the target
#   ./scripts/dump_cli_reference.sh --dump   print the raw dump to stdout
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=docs/aqpbm-cli-reference.md

cargo build --bin approxbench >&2
B=target/debug/approxbench

# Every subcommand path the reference documents, deepest last. Paths the
# binary does not implement yet are reported as missing, which is the gap.
SUBCOMMANDS=(
    "sketchbench"
    "sketchprofile"
    "dataset"
    "dataset generate"
    "dataset describe"
)

dump() {
    echo '## `approxbench`'
    echo
    echo '```'
    "$B" --help
    echo '```'
    echo
    for sub in "${SUBCOMMANDS[@]}"; do
        echo "## \`approxbench $sub\`"
        echo
        echo '```'
        # shellcheck disable=SC2086
        "$B" $sub --help 2>&1 || echo "(not implemented)"
        echo '```'
        echo
    done
}

if [[ "${1:-}" == "--dump" ]]; then
    dump
    exit 0
fi

ACTUAL=$(mktemp)
trap 'rm -f "$ACTUAL"' EXIT
dump >"$ACTUAL"

# Compare headings and fenced blocks only. The target's prose between blocks is
# commentary the binary has no way to produce.
fenced_only() {
    awk '
        /^## `approxbench/ { print; next }
        /^```/             { infence = !infence; print; next }
        infence            { print }
    ' "$1"
}

EXPECTED=$(mktemp)
trap 'rm -f "$ACTUAL" "$EXPECTED"' EXIT
fenced_only "$TARGET" >"$EXPECTED"
fenced_only "$ACTUAL" >"$ACTUAL.f" && mv "$ACTUAL.f" "$ACTUAL"

if diff -u "$EXPECTED" "$ACTUAL"; then
    echo "$TARGET matches the built binary"
else
    echo
    echo "the binary does not match $TARGET yet (diff above: - target, + binary)"
    exit 1
fi
