#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
from collections import defaultdict
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

COLORS = {
    "rust_datasketches_cms": "#54A24B",
    "rust_sketchlib_cms": "#4C78A8",
    "cpp_datasketches_cms": "#F58518",
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        rows = list(reader)
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def plot_mean_with_error_bars(rows: list[dict[str, str]], output_path: Path) -> None:
    grouped: dict[str, dict[int, list[float]]] = defaultdict(lambda: defaultdict(list))
    for row in rows:
        grouped[row["implementation"]][int(row["cols"])].append(float(row["avg_relative_error"]))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(9, 5.5))
    x_values = [2048, 4096, 8192]

    for implementation, per_col in grouped.items():
        means = []
        lower = []
        upper = []
        for col in x_values:
            values = per_col[col]
            mean = sum(values) / len(values)
            means.append(mean)
            lower.append(mean - min(values))
            upper.append(max(values) - mean)
        ax.errorbar(
            x_values,
            means,
            yerr=[lower, upper],
            marker="o",
            linewidth=2,
            capsize=5,
            color=COLORS.get(implementation, "#333333"),
            label=implementation,
        )

    ax.set_title("CMS Accuracy: Mean Relative Error Across Seeds")
    ax.set_xlabel("Columns")
    ax.set_ylabel("Average Relative Error")
    ax.set_xticks(x_values)
    ax.grid(True, axis="y", alpha=0.25)
    ax.legend()
    fig.tight_layout()
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def plot_seed_scatter(rows: list[dict[str, str]], output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(9, 5.5))
    x_values = [2048, 4096, 8192]
    offsets = {
        "rust_datasketches_cms": -80,
        "rust_sketchlib_cms": 0,
        "cpp_datasketches_cms": 80,
    }

    for implementation in sorted({row["implementation"] for row in rows}):
        xs = []
        ys = []
        for row in rows:
            if row["implementation"] != implementation:
                continue
            xs.append(int(row["cols"]) + offsets.get(implementation, 0))
            ys.append(float(row["avg_relative_error"]))
        ax.scatter(xs, ys, label=implementation, alpha=0.75, color=COLORS.get(implementation, "#333333"))

    ax.set_title("CMS Accuracy: Per-Seed Relative Error")
    ax.set_xlabel("Columns")
    ax.set_ylabel("Average Relative Error")
    ax.set_xticks(x_values)
    ax.grid(True, axis="y", alpha=0.25)
    ax.legend()
    fig.tight_layout()
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--scatter-output", required=True, type=Path)
    args = parser.parse_args()

    rows = load_rows(args.input)
    plot_mean_with_error_bars(rows, args.output)
    plot_seed_scatter(rows, args.scatter_output)


if __name__ == "__main__":
    main()
