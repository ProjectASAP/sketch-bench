#!/usr/bin/env python3
"""A guided tour of `--config`: how to build a sketch at the parameters you name.

There is no built-in configuration to override. Every invocation states its own
construction point, and the row either builds at exactly that point or refuses
it and says what bound it has. Nothing is clamped to a library's range, rounded
to a shape it prefers, or dropped because the row has no knob for it.

That is the whole contract, and this script demonstrates it end to end:

  1. a run states its own construction point, and the record carries it back
  2. the knob moves the sketch, so a sweep measures something
  3. an odd point is honoured exactly, not snapped to a convenient one
  4. a point the library cannot honour is refused by name
  5. one point crosses every library and variant in a family

Usage:

    scripts/example_config_override.py              # the whole tour
    scripts/example_config_override.py 2 4          # only those sections
    scripts/example_config_override.py --size 1000000

The workload is small by default so the tour finishes in under a minute. It is
a demonstration of the parameter surface, not a measurement: read the numbers
for whether they move, not for what they are.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
BIN = REPO / "target" / "release" / "approxbench"

# Which accuracy metric to show per family. Each family is scored by its own
# comparator, so there is no one column that means the same thing across them.
ERROR_KEY = {
    "hll": "relative_error",
    "kll": "mean_rank_err",
    "cms": "relative_error_mean",
    "countsketch": "relative_error_mean",
    "hydra-cms": "relative_error_mean",
    "hydra-hll": "relative_error_mean",
    "hydra-kll": "mean_rank_err",
}

# The rows that ingest labelled records read a column list instead of a single
# stream, so they need a spec file. `--input` and the inline flags cannot
# describe one.
HYDRA_SPEC = REPO / "configs" / "datagen" / "hydra_columns.yaml"


class Result:
    """What one invocation produced: a built cell, or the refusal it printed."""

    def __init__(self, record=None, error=None):
        self.record = record
        self.error = error

    @property
    def ok(self) -> bool:
        return self.record is not None

    def cell(self, family: str) -> str:
        """One line of numbers, or the refusal, whichever the run produced."""
        if not self.ok:
            return f"REFUSED  {self.error}"
        bench = self.record["bench"]
        memory = bench.get("memory_bytes")
        accuracy = bench.get("accuracy") or {}
        err = accuracy.get(ERROR_KEY.get(family, ""))
        parts = [f"memory={memory:>10,} B" if memory is not None else " " * 20]
        if err is not None:
            parts.append(f"error={err:<12.6g}")
        return "  ".join(parts)


def run(algorithm: str, impl: str, config: str, args) -> Result:
    """One cell: one algorithm, one impl, one construction point."""
    cmd = [
        str(BIN), "sketchbench",
        "--variant", algorithm,
        "--library", impl,
        "--config", config,
        "--runs", "1",
        "--warmup-runs", "0",
        "--accuracy",
        "--metrics", "accuracy",
    ]
    if algorithm.startswith("hydra"):
        cmd += ["--spec", str(HYDRA_SPEC)]
    else:
        cmd += [
            "--size", str(args.size),
            "--cardinality", str(args.cardinality),
        ]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    for line in proc.stdout.splitlines():
        if line.startswith("{"):
            record = json.loads(line)
            # The accuracy pass is the one carrying both the footprint and the
            # comparator's verdict; the timed passes carry neither.
            if (record.get("bench") or {}).get("pass") == "accuracy":
                return Result(record=record)
    # A refusal is the point of half this script, so it is a result and not a
    # crash. The wrapper's own message is the last line of stderr.
    message = next(
        (l for l in reversed(proc.stderr.splitlines()) if l.startswith("Error:")),
        proc.stderr.strip() or "(no output)",
    )
    return Result(error=message.replace("Error: ", "", 1))


def show(algorithm: str, impl: str, config: str, args, note: str = "") -> Result:
    result = run(algorithm, impl, config, args)
    family = result.record["algorithm"] if result.ok else _family_guess(algorithm)
    label = f"{algorithm}/{impl}"
    print(f"    {label:<46} {config:<40} {result.cell(family)}")
    if note:
        print(f"    {'':46} {note}")
    return result


def _family_guess(algorithm: str) -> str:
    """The family of a row that did not build, for picking its error column."""
    for family in sorted(ERROR_KEY, key=len, reverse=True):
        if algorithm == family or algorithm.startswith(family + "-"):
            return family
    return algorithm


def heading(number: int, title: str, body: str) -> None:
    print()
    print(f"=== {number}. {title} " + "=" * max(0, 76 - len(title)))
    print()
    for line in body.strip().split("\n"):
        print(f"  {line}")
    print()


# --------------------------------------------------------------------------
# the tour
# --------------------------------------------------------------------------


def section_1(args) -> None:
    heading(1, "a run states its own construction point", """
A cell is one algorithm, one impl, one construction point, one workload.
`--config` is that point, and it is not optional: omitting it is an error
naming the first field the row needs.

The record carries the point back in `sketch_config`, so a reader never has to
guess what a number was measured at.
""")
    show("hll", "oxide", "lg_k=11", args)
    result = run("hll", "oxide", "lg_k=11", args)
    if result.ok:
        print()
        print("    the record says, verbatim:")
        for field in ("algorithm", "sketch", "impl", "sketch_config"):
            print(f"      {field:<14} {json.dumps(result.record[field])}")
    print()
    print("    omitting it is refused, naming the field the row needs:")
    proc = subprocess.run(
        [str(BIN), "sketchbench", "--variant", "hll", "--library", "oxide",
         "--size", "1000", "--runs", "1", "--warmup-runs", "0",
         "--metrics", "throughput"],
        capture_output=True, text=True,
    )
    print(f"      {proc.stderr.strip().splitlines()[-1]}")


def section_2(args) -> None:
    heading(2, "the knob moves the sketch", """
The parameter reaches the structure, so a sweep measures something: more space
buys less error. If a knob were being dropped, every row of a sweep would come
back with one footprint and one error under three different labels, which is
exactly the defect this contract exists to prevent.
""")
    print("  HyperLogLog: `lg_k` is the log2 register count.")
    for lg_k in (12, 14, 16):
        show("hll", "lib", f"lg_k={lg_k}", args)
    print()
    print("  KLL: `k` sizes the compactors.")
    for k in (8, 137, 4096):
        show("kll-percall", "lib", f"k={k}", args)
    print()
    print("  Count-Min: `rows` x `cols` is the counter matrix.")
    for cols in (512, 2048, 8192):
        show("cms", "oxide", f"rows=5 cols={cols}", args)


def section_3(args) -> None:
    heading(3, "an odd point is honoured exactly", """
Nothing here is a round number, and nothing is snapped to one. A point is legal
whenever the library can build at it, and whether it is a sensible point to
measure is the caller's business.

Read the footprints: they are what the requested dimensions predict, not what a
convenient nearby shape would give.
""")
    show("cms", "oxide", "rows=11 cols=1024", args,
         note="11 x 1024 x 8 B = 90,112")
    show("countsketch", "oxide", "rows=7 cols=4096", args,
         note="7 x 4096 x 8 B = 229,376")
    show("kll-percall", "lib", "k=613", args)
    show("hll", "oxide", "lg_k=17", args, note="2^17 = 131,072 registers")
    print()
    print("  The grouped rows take two nested shapes, and both are honoured.")
    show("hydra-cms", "lib", "rows=7 cols=33 cell_rows=2 cell_cols=257", args)
    show("hydra-kll", "lib", "rows=5 cols=37 cell_k=613", args)


def section_4(args) -> None:
    heading(4, "a point the library cannot honour is refused", """
Every one of these used to build. The sketch ran at some other parameter and the
record named the one that was asked for, so a plot keyed on `sketch_config` put
one measurement at several x-positions, or several measurements at one.

A refusal names the bound, so the next invocation can be right.
""")
    show("hll", "lib", "lg_k=13", args)
    show("kll-percall", "lib", "k=4", args)
    show("kll-percall", "lib", "k=40000", args)
    show("cms", "oxide", "rows=5 cols=3000", args)
    show("countsketch", "oxide", "rows=2 cols=2048", args)
    show("cms-fastpath-fixedmatrix", "lib", "rows=5 cols=4096", args)
    show("hydra-kll", "lib", "rows=3 cols=64 cell_k=1", args)
    print()
    print("  A misspelled key is refused the same way, on a sketch row and on")
    print("  the exact baseline it is scored against alike:")
    show("cms", "oxide", "rows=5 colz=2048", args)
    show("cms", "polars", "rows=5 colz=2048", args)


def section_5(args) -> None:
    heading(5, "one point crosses every library and variant", """
`--library` is the library and nothing else, so grouping on it answers "which
library implements this best". `--variant` carries the structural detail, so
grouping on it answers "which variant of this structure wins". The `family`
field groups the variants back together, and it is what a cross-library
comparison is taken over.

One `--config` means the same 5 x 2048 counters everywhere, which is what makes
the comparison a comparison. The byte figures still differ, because a counter is
4 bytes in `asap_sketchlib` and 8 in the other two: that is a choice each
library made, and the footprint column reports it instead of hiding it. The
`polars` row holds the whole stream, since an exact answer is what it is for.
""")
    config = "rows=5 cols=2048"
    for algorithm, impl in (
        ("cms", "oxide"),
        ("cms", "datasketches"),
        ("cms", "polars"),
        ("cms-fastpath-vector2d", "lib"),
        ("cms-regularpath-vector2d", "lib"),
        ("cms-fastpath-fixedmatrix", "lib"),
        ("countsketch", "oxide"),
        ("countsketch-fastpath-vector2d", "lib"),
    ):
        show(algorithm, impl, config, args)
    print()
    print("  `--list-impls` prints every pair, grouped by family.")


SECTIONS = [section_1, section_2, section_3, section_4, section_5]


def main() -> int:
    parser = argparse.ArgumentParser(
        description="A guided tour of approxbench's --config surface.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("sections", nargs="*", type=int,
                        help="which sections to run (default: all)")
    parser.add_argument("--size", type=int, default=200_000,
                        help="workload size (default: 200000)")
    parser.add_argument("--cardinality", type=int, default=50_000,
                        help="distinct keys in the workload (default: 50000)")
    args = parser.parse_args()

    if not BIN.exists():
        print(f"building {BIN.name}...", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release", "-p", "aqpbm-cli"],
            cwd=REPO, check=True,
        )

    chosen = args.sections or range(1, len(SECTIONS) + 1)
    for n in chosen:
        if not 1 <= n <= len(SECTIONS):
            print(f"no section {n}; there are {len(SECTIONS)}", file=sys.stderr)
            return 2
        SECTIONS[n - 1](args)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
