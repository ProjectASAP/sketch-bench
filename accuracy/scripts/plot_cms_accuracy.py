#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
from collections import defaultdict
from matplotlib.lines import Line2D
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


def plot_grouped_boxplots(rows: list[dict[str, str]], output_path: Path) -> None:
    grouped: dict[str, dict[int, list[float]]] = defaultdict(lambda: defaultdict(list))
    for row in rows:
        grouped[row["implementation"]][int(row["cols"])].append(
            float(row["median_relative_error"]) * 100.0
        )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(12.5, 5.5))
    x_values = [2048, 4096, 8192, 16384, 32768, 65536, 131072]
    implementations = ["rust_datasketches_cms", "rust_sketchlib_cms", "cpp_datasketches_cms"]
    offsets = {
        "rust_datasketches_cms": -0.24,
        "rust_sketchlib_cms": 0.0,
        "cpp_datasketches_cms": 0.24,
    }
    width = 0.18

    for implementation in implementations:
        positions = [index + offsets[implementation] for index in range(len(x_values))]
        data = [grouped[implementation][col] for col in x_values]
        bp = ax.boxplot(
            data,
            positions=positions,
            widths=width,
            patch_artist=True,
            showfliers=False,
            medianprops={"color": "#222222", "linewidth": 2},
            whiskerprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
            capprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
            boxprops={"facecolor": COLORS.get(implementation, "#333333"), "edgecolor": COLORS.get(implementation, "#333333"), "alpha": 0.75},
        )
        for patch in bp["boxes"]:
            patch.set_facecolor(COLORS.get(implementation, "#333333"))
            patch.set_edgecolor(COLORS.get(implementation, "#333333"))
            patch.set_alpha(0.75)

        medians = []
        for values in data:
            sorted_values = sorted(values)
            medians.append(sorted_values[len(sorted_values) // 2])
        ax.plot(
            positions,
            medians,
            color=COLORS.get(implementation, "#333333"),
            linewidth=2,
            marker="o",
            markersize=5,
            zorder=3,
        )

    ax.set_title(
        "CMS Accuracy: Per-Key Median-Over-Seeds Relative Error\n"
        "Data: 1M uniformly random int64 values\n"
        "Sketch fixed to 5 rows"
    )
    ax.set_xlabel("Columns")
    ax.set_ylabel("Median-Over-Seeds Relative Error (%)")
    ax.set_xticks(range(len(x_values)))
    ax.set_xticklabels([str(value) for value in x_values])
    ax.grid(True, axis="y", alpha=0.25)
    legend_handles = [
        Line2D([0], [0], color=COLORS[name], lw=6, label=name) for name in implementations
    ]
    ax.legend(handles=legend_handles)
    fig.tight_layout()
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input-key-errors", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    rows = load_rows(args.input_key_errors)
    plot_grouped_boxplots(rows, args.output)


if __name__ == "__main__":
    main()
