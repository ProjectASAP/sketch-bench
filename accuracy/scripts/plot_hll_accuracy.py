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
from matplotlib.lines import Line2D

COLORS = {
    "rust_sketchlib_hll": "#4C78A8",
    "rust_datasketches_hll": "#54A24B",
    "cpp_datasketches_hll": "#F58518",
}
IMPLEMENTATIONS = [
    "rust_sketchlib_hll",
    "rust_datasketches_hll",
    "cpp_datasketches_hll",
]
OFFSETS = {
    "rust_sketchlib_hll": -0.24,
    "rust_datasketches_hll": 0.0,
    "cpp_datasketches_hll": 0.24,
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def group_rows(rows: list[dict[str, str]]) -> tuple[dict[str, list[float]], str]:
    grouped: dict[str, list[float]] = defaultdict(list)
    labels = set()
    for row in rows:
        grouped[row["implementation"]].append(float(row["relative_error"]) * 100.0)
        labels.add(f"lg_k={row['lg_k']} ({row['registers']} registers)")
    if len(labels) != 1:
        raise ValueError(f"expected exactly one fixed-size HLL group, found: {sorted(labels)}")
    return grouped, labels.pop()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    rows = load_rows(args.input)
    grouped, group_label = group_rows(rows)
    for implementation in IMPLEMENTATIONS:
        if not grouped[implementation]:
            raise ValueError(f"missing rows for implementation={implementation}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(8.8, 5.6))
    center = 0.0
    width = 0.18

    for implementation in IMPLEMENTATIONS:
        position = center + OFFSETS[implementation]
        values = grouped[implementation]
        bp = ax.boxplot(
            [values],
            positions=[position],
            widths=width,
            patch_artist=True,
            showfliers=False,
            medianprops={"color": "#222222", "linewidth": 2},
            whiskerprops={"color": COLORS[implementation], "linewidth": 1.5},
            capprops={"color": COLORS[implementation], "linewidth": 1.5},
            boxprops={"facecolor": COLORS[implementation], "edgecolor": COLORS[implementation], "alpha": 0.78},
        )
        for patch in bp["boxes"]:
            patch.set_facecolor(COLORS[implementation])
            patch.set_edgecolor(COLORS[implementation])
            patch.set_alpha(0.78)
        median = sorted(values)[len(values) // 2]
        ax.plot(
            [position],
            [median],
            color=COLORS[implementation],
            marker="o",
            markersize=5,
            linewidth=0,
            zorder=3,
        )

    ax.set_title(
        "\n".join(
            [
                "HLL Accuracy: Relative Error Over 10 Seeded Input Remappings",
                "Data: 1M Zipf-distributed int64 values (s=1.1, support=100k)",
                "Single fixed-size group in Sketchlib Rust and DataSketches",
            ]
        )
    )
    ax.set_xlabel("Configuration")
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks([center])
    ax.set_xticklabels([group_label])
    ax.grid(True, axis="y", alpha=0.25)
    ax.legend(
        handles=[Line2D([0], [0], color=COLORS[name], lw=6, label=name) for name in IMPLEMENTATIONS]
    )
    fig.tight_layout()
    fig.savefig(args.output, dpi=200)
    plt.close(fig)


if __name__ == "__main__":
    main()
