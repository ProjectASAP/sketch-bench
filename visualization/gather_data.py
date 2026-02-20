#!/usr/bin/env python3
"""
Collects benchmark run data from ../benchmark_*/output/*.txt files and writes
per-benchmark CSV files with one column per implementation and ten rows for runs
0..9. Run from the visualization directory.
"""

from __future__ import annotations

import csv
import re
import sys
from collections import OrderedDict
from pathlib import Path
from typing import Iterable, List, MutableMapping

EXPECTED_RUNS = 10
RUN_RE = re.compile(r"Run\s+(?P<index>\d+):\s+(?P<value>[\d,]+(?:\.\d+)?)")


def find_benchmark_dirs(repo_root: Path) -> Iterable[Path]:
    for path in sorted(repo_root.glob("benchmark_*")):
        if (path / "output").is_dir():
            yield path


def parse_results_file(path: Path) -> "OrderedDict[str, List[float]]":
    """
    Returns an ordered mapping of {label -> [run0, run1, ...]} for a single
    result text file.
    """
    experiments: "OrderedDict[str, List[float]]" = OrderedDict()
    bench_name = path.parent.parent.name
    default_label = f"{bench_name}::{path.stem}"
    current_label: str | None = None
    pending_label: str | None = None

    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.endswith(":") and not RUN_RE.match(line):
            pending_label = line[:-1].strip()
            continue
        match = RUN_RE.search(line)
        if not match:
            continue

        label = pending_label or current_label or default_label
        pending_label = None
        if label not in experiments:
            experiments[label] = [None] * EXPECTED_RUNS  # type: ignore[list-item]
        current_label = label

        run_idx = int(match.group("index"))
        if not 0 <= run_idx < EXPECTED_RUNS:
            raise ValueError(
                f"{path}: unexpected run index {run_idx}; expected 0-{EXPECTED_RUNS - 1}"
            )
        value_str = match.group("value").replace(",", "")
        value = float(value_str) if "." in value_str else int(value_str)
        experiments[label][run_idx] = value  # type: ignore[index]

    for label, runs in experiments.items():
        if any(v is None for v in runs):
            raise ValueError(f"{path}: Missing runs for {label}")

    return experiments


def dedupe_label(label: str, existing: MutableMapping[str, List[float]], hint: str) -> str:
    if label not in existing:
        return label
    base = f"{label} ({hint})"
    if base not in existing:
        return base
    idx = 2
    while True:
        candidate = f"{base}-{idx}"
        if candidate not in existing:
            return candidate
        idx += 1


def gather_benchmark(bench_dir: Path) -> "OrderedDict[str, List[float]]":
    experiments: "OrderedDict[str, List[float]]" = OrderedDict()
    output_dir = bench_dir / "output"
    for txt_file in sorted(output_dir.glob("*.txt")):
        per_file = parse_results_file(txt_file)
        for label, runs in per_file.items():
            unique_label = dedupe_label(label, experiments, txt_file.stem)
            experiments[unique_label] = runs
    return experiments


def write_csv(csv_path: Path, experiments: "OrderedDict[str, List[float]]") -> None:
    fieldnames = ["run"] + list(experiments.keys())
    rows = []
    for run_idx in range(EXPECTED_RUNS):
        row = {"run": run_idx}
        for label, runs in experiments.items():
            row[label] = runs[run_idx]
        rows.append(row)

    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent
    data_dir = script_dir / "data"
    generated = 0

    for bench_dir in find_benchmark_dirs(repo_root):
        experiments = gather_benchmark(bench_dir)
        if not experiments:
            continue
        csv_path = data_dir / f"{bench_dir.name}.csv"
        write_csv(csv_path, experiments)
        generated += 1
        print(
            f"Wrote {csv_path.relative_to(repo_root)} "
            f"with {len(experiments)} implementation columns."
        )

    if generated == 0:
        print("No benchmark output files found.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
