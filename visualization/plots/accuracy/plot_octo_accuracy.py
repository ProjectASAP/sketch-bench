#!/usr/bin/env python3
"""Plot OctoSketch accuracy for CMS, CS, and HLL across thread counts."""
from __future__ import annotations

import argparse
import csv
import os
import statistics
from collections import defaultdict
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

THREAD_COLORS = {1: "#4C78A8", 2: "#54A24B", 4: "#F58518", 8: "#E45756"}
THREAD_COUNTS = [1, 2, 4, 8]
DATASET_LABEL = os.environ.get(
    "ACCURACY_DATASET_LABEL",
    "Data: 10M Zipf-distributed int64 values (s=1.1, support=100k)",
)


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def plot_cms_cs(rows: list[dict[str, str]], output: Path, sketch_label: str) -> None:
    grouped: dict[int, list[float]] = defaultdict(list)
    total_items = set()
    for row in rows:
        grouped[int(row["num_workers"])].append(float(row["avg_relative_error"]) * 100)
        total_items.add(int(row["total_items"]))

    labels = [f"Octo {t}T" for t in THREAD_COUNTS]
    medians = [statistics.median(grouped[t]) for t in THREAD_COUNTS]
    positions = list(range(len(THREAD_COUNTS)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(9.2, 5.6))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=[THREAD_COLORS[t] for t in THREAD_COUNTS],
        alpha=0.88,
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[t] for t in THREAD_COUNTS],
        positions=positions,
        widths=0.22,
        patch_artist=True,
        showfliers=False,
        zorder=3,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"linewidth": 1.4},
        capprops={"linewidth": 1.4},
        boxprops={"linewidth": 1.2},
    )
    for patch, t in zip(box["boxes"], THREAD_COUNTS):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(THREAD_COLORS[t])
        patch.set_alpha(0.96)
    for whisker, t in zip(
        box["whiskers"], [t for t in THREAD_COUNTS for _ in range(2)]
    ):
        whisker.set_color(THREAD_COLORS[t])
    for cap, t in zip(box["caps"], [t for t in THREAD_COUNTS for _ in range(2)]):
        cap.set_color(THREAD_COLORS[t])

    ax.set_title(
        "\n".join(
            [
                f"OctoSketch {sketch_label} Accuracy (Avg Relative Error % on Heavy Hitters)",
                DATASET_LABEL,
                "5 x 32,768 sketch; 10 runs per config",
            ]
        )
    )
    ax.set_ylabel("Avg Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels(labels)
    ax.grid(True, axis="y", alpha=0.25, zorder=1)

    max_height = max(max(v) for v in grouped.values()) if grouped else 1
    ax.set_ylim(0, max_height * 1.25)
    for bar, value in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{value:.2f}%",
            ha="center",
            va="bottom",
            fontsize=10,
        )

    fig.tight_layout()
    fig.savefig(output, dpi=200)
    plt.close(fig)


def plot_hll(rows: list[dict[str, str]], output: Path) -> None:
    grouped: dict[int, list[float]] = defaultdict(list)
    total_items = set()
    for row in rows:
        grouped[int(row["num_workers"])].append(float(row["relative_error"]) * 100)
        total_items.add(int(row["total_items"]))

    labels = [f"Octo {t}T" for t in THREAD_COUNTS]
    medians = [statistics.median(grouped[t]) for t in THREAD_COUNTS]
    positions = list(range(len(THREAD_COUNTS)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(9.2, 5.6))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=[THREAD_COLORS[t] for t in THREAD_COUNTS],
        alpha=0.88,
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[t] for t in THREAD_COUNTS],
        positions=positions,
        widths=0.22,
        patch_artist=True,
        showfliers=False,
        zorder=3,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"linewidth": 1.4},
        capprops={"linewidth": 1.4},
        boxprops={"linewidth": 1.2},
    )
    for patch, t in zip(box["boxes"], THREAD_COUNTS):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(THREAD_COLORS[t])
        patch.set_alpha(0.96)
    for whisker, t in zip(
        box["whiskers"], [t for t in THREAD_COUNTS for _ in range(2)]
    ):
        whisker.set_color(THREAD_COLORS[t])
    for cap, t in zip(box["caps"], [t for t in THREAD_COUNTS for _ in range(2)]):
        cap.set_color(THREAD_COLORS[t])

    ax.set_title(
        "\n".join(
            [
                "OctoSketch HLL Accuracy (Cardinality Relative Error %)",
                DATASET_LABEL,
                "HLL P14 (16,384 registers); 10 runs per config",
            ]
        )
    )
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels(labels)
    ax.grid(True, axis="y", alpha=0.25, zorder=1)

    max_height = max(max(v) for v in grouped.values()) if grouped else 1
    ax.set_ylim(0, max_height * 1.25)
    for bar, value in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{value:.2f}%",
            ha="center",
            va="bottom",
            fontsize=10,
        )

    fig.tight_layout()
    fig.savefig(output, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input-cms", required=True, type=Path)
    parser.add_argument("--input-cs", required=True, type=Path)
    parser.add_argument("--input-hll", required=True, type=Path)
    parser.add_argument("--output-cms", required=True, type=Path)
    parser.add_argument("--output-cs", required=True, type=Path)
    parser.add_argument("--output-hll", required=True, type=Path)
    args = parser.parse_args()

    plot_cms_cs(load_rows(args.input_cms), args.output_cms, "CMS")
    print(f"Wrote {args.output_cms}")
    plot_cms_cs(load_rows(args.input_cs), args.output_cs, "CS")
    print(f"Wrote {args.output_cs}")
    plot_hll(load_rows(args.input_hll), args.output_hll)
    print(f"Wrote {args.output_hll}")


if __name__ == "__main__":
    main()
