#!/usr/bin/env python3
"""
Cross-track orchestrator: run the Rust `sketchlib bench` and the C++
`cpp-bench/<sketch>/<impl>_<sketch>` binaries against a shared
workload file, concatenate every emitted v1 JSONL record into one
report file.

The two tracks never speak to each other directly — they only have
to agree on the v1 wire format (see docs/SCHEMA_V1.md). This script
is just a fan-out + fan-in over `subprocess`.

Examples
--------
Run KLL on both tracks with the default 1 M workload:

    scripts/run_all.py \\
        --workload-file input/benchmark_data_1m_int64.bin \\
        --sketches kll \\
        --report reports/$(date +%F).jsonl

Include accuracy for everything:

    scripts/run_all.py \\
        --workload-file input/benchmark_data_1m_int64.bin \\
        --sketches kll,cs \\
        --with-accuracy \\
        --report reports/$(date +%F).jsonl
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


REPO_ROOT = Path(__file__).resolve().parent.parent


# Combos to run by default. Each entry is (track, sketch, impl).
# `sketch` matches what each track's `Record.sketch` field will be
# (e.g. Rust dispatch uses "countsketch", not "cs"). C++ binaries
# emit the same family name to keep the JSONL apples-to-apples.
#
# C++ binary path convention: `{cpp_bin_dir}/{cpp_dir}/{impl}_{cpp_dir}`,
# where `cpp_dir` is the on-disk dir name in cpp-bench/ (which for the
# countsketch family is "cs").
DEFAULT_COMBOS: list[tuple[str, str, str]] = [
    # Rust track — names from sketch-cli/src/dispatch.rs
    ("rust", "kll",         "lib"),
    ("rust", "kll",         "oxide"),
    ("rust", "countsketch", "oxide"),
    ("rust", "countsketch", "lib-fixedmatrix-fast"),
    # C++ track — names from cpp-bench/<dir>/<impl>_<dir>
    ("cpp",  "kll",         "datasketches"),
    ("cpp",  "kll",         "final"),
    ("cpp",  "countsketch", "datasketches"),
    ("cpp",  "countsketch", "final"),
]

# C++ family name → on-disk directory under cpp-bench/. Most match
# (kll → kll), but the countsketch family lives under cs/.
CPP_DIR_FOR_SKETCH: dict[str, str] = {
    "kll": "kll",
    "countsketch": "cs",
}


@dataclass
class Args:
    workload_file: Path
    sketches: set[str] | None
    impls: set[str] | None
    tracks: set[str]
    runs: int
    measure_items: int | None
    warmup_items: int | None
    latency_stride: int | None
    with_accuracy: bool
    report: Path
    rust_bin: Path
    cpp_bin_dir: Path
    dry_run: bool


def parse_args(argv: list[str]) -> Args:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--workload-file", type=Path, required=True,
                   help="little-endian int64 .bin file fed to both tracks")
    p.add_argument("--sketches", default=None,
                   help="comma list filter, e.g. 'kll,cs' (default: all)")
    p.add_argument("--impls", default=None,
                   help="comma list filter, e.g. 'datasketches,final' (default: all)")
    p.add_argument("--tracks", default="rust,cpp",
                   help="comma list, subset of {rust,cpp} (default: both)")
    p.add_argument("--runs", type=int, default=10)
    p.add_argument("--measure-items", type=int, default=None)
    p.add_argument("--warmup-items", type=int, default=None)
    p.add_argument("--latency-stride", type=int, default=None,
                   help="C++ track only; Rust uses its own latency sampler")
    p.add_argument("--with-accuracy", action="store_true")
    p.add_argument("--report", type=Path, required=True,
                   help="JSONL file every record is appended to")
    p.add_argument("--rust-bin", type=Path,
                   default=REPO_ROOT / "target" / "release" / "sketchlib",
                   help="path to the sketchlib binary (default: cargo release build)")
    p.add_argument("--cpp-bin-dir", type=Path,
                   default=REPO_ROOT / "cpp-bench" / "build",
                   help="directory containing the cpp-bench binaries")
    p.add_argument("--dry-run", action="store_true",
                   help="print the commands that would run, don't execute")
    ns = p.parse_args(argv)

    def split_csv(v: str | None) -> set[str] | None:
        if v is None:
            return None
        return {t.strip() for t in v.split(",") if t.strip()}

    return Args(
        workload_file=ns.workload_file.resolve(),
        sketches=split_csv(ns.sketches),
        impls=split_csv(ns.impls),
        tracks={t.strip() for t in ns.tracks.split(",") if t.strip()},
        runs=ns.runs,
        measure_items=ns.measure_items,
        warmup_items=ns.warmup_items,
        latency_stride=ns.latency_stride,
        with_accuracy=ns.with_accuracy,
        report=ns.report.resolve(),
        rust_bin=ns.rust_bin.resolve() if ns.rust_bin else ns.rust_bin,
        cpp_bin_dir=ns.cpp_bin_dir.resolve(),
        dry_run=ns.dry_run,
    )


def select_combos(args: Args) -> list[tuple[str, str, str]]:
    out = []
    for track, sketch, impl in DEFAULT_COMBOS:
        if track not in args.tracks:
            continue
        if args.sketches is not None and sketch not in args.sketches:
            continue
        if args.impls is not None and impl not in args.impls:
            continue
        out.append((track, sketch, impl))
    return out


def rust_cmd(args: Args, sketch: str, impl: str) -> list[str]:
    cmd = [
        str(args.rust_bin), "bench",
        "--sketch", sketch,
        "--impl", impl,
        "--runs", str(args.runs),
        "--input", str(args.workload_file),
    ]
    if args.with_accuracy:
        cmd.append("--accuracy")
    return cmd


def cpp_cmd(args: Args, sketch: str, impl: str) -> list[str]:
    cpp_dir = CPP_DIR_FOR_SKETCH.get(sketch, sketch)
    bin_path = args.cpp_bin_dir / cpp_dir / f"{impl}_{cpp_dir}"
    cmd = [
        str(bin_path),
        "--workload-file", str(args.workload_file),
        "--runs", str(args.runs),
    ]
    if args.measure_items is not None:
        cmd.extend(["--measure-items", str(args.measure_items)])
    if args.warmup_items is not None:
        cmd.extend(["--warmup-items", str(args.warmup_items)])
    if args.latency_stride is not None:
        cmd.extend(["--latency-stride", str(args.latency_stride)])
    if args.with_accuracy:
        cmd.append("--with-accuracy")
    return cmd


def run_one(cmd: list[str], track: str) -> Iterable[str]:
    """Run a benchmark binary, yield its stdout lines.

    Lines that aren't valid JSON (warnings, gbench banner) go to stderr
    and are skipped — only well-formed JSONL records make it through.
    """
    print(f"[run_all] $ {' '.join(cmd)}", file=sys.stderr)
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        print(f"[run_all] {track} command exit {proc.returncode}:\n{proc.stderr}",
              file=sys.stderr)
        return
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            json.loads(line)
        except json.JSONDecodeError:
            print(f"[run_all] ({track}) ignoring non-JSON stdout: {line[:120]}",
                  file=sys.stderr)
            continue
        yield line


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    combos = select_combos(args)
    if not combos:
        print("[run_all] no combos selected after filters", file=sys.stderr)
        return 1

    if not args.workload_file.is_file():
        print(f"[run_all] workload file not found: {args.workload_file}", file=sys.stderr)
        return 2

    # Sanity check the binaries exist (skip in dry-run).
    if not args.dry_run:
        if any(t == "rust" for (t, _, _) in combos) and not args.rust_bin.is_file():
            print(f"[run_all] sketchlib not found at {args.rust_bin}; "
                  "run `cargo build --release -p sketch-cli` first", file=sys.stderr)
            return 2
        if any(t == "cpp" for (t, _, _) in combos) and not args.cpp_bin_dir.is_dir():
            print(f"[run_all] cpp-bench build dir not found at {args.cpp_bin_dir}; "
                  "build cpp-bench/ first", file=sys.stderr)
            return 2

    args.report.parent.mkdir(parents=True, exist_ok=True)
    n_records = 0
    with open(args.report, "a") as out:
        for track, sketch, impl in combos:
            cmd = rust_cmd(args, sketch, impl) if track == "rust" else cpp_cmd(args, sketch, impl)
            if args.dry_run:
                print(" ".join(cmd))
                continue
            for line in run_one(cmd, track):
                out.write(line + "\n")
                out.flush()
                n_records += 1

    print(f"[run_all] wrote {n_records} records to {args.report}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
