#!/usr/bin/env python3
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

COLORS = {
    "rust_sketchlib_cms": "#4C78A8",
    "rust_oxide_cms": "#54A24B",
    "rust_datasketches_cms": "#E45756",
    "cpp_datasketches_cms": "#F58518",
}

LABELS = {
    "rust_sketchlib_cms": "Rust sketchlib",
    "rust_oxide_cms": "Rust sketch_oxide",
    "rust_datasketches_cms": "Rust DataSketches",
    "cpp_datasketches_cms": "C++ DataSketches",
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(rows: list[dict[str, str]], output: Path) -> None:
    grouped: dict[str, list[float]] = defaultdict(list)
    cols = set()
    seed_count = set()
    total_items = set()
    for row in rows:
        grouped[row["implementation"]].append(float(row["throughput_items_per_sec"]))
        cols.add(int(row["cols"]))
        seed_count.add(int(row["seed"]))
        total_items.add(int(row["total_items"]))

    implementations = [
        "rust_sketchlib_cms",
        "rust_oxide_cms",
        "rust_datasketches_cms",
        "cpp_datasketches_cms",
    ]
    for implementation in implementations:
        if not grouped.get(implementation):
            raise ValueError(f"missing data for {implementation}")

    medians = [statistics.median(grouped[implementation]) for implementation in implementations]
    positions = list(range(len(implementations)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 5.8))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=[COLORS[implementation] for implementation in implementations],
        alpha=0.88,
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[implementation] for implementation in implementations],
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
    for patch, implementation in zip(box["boxes"], implementations):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(COLORS[implementation])
        patch.set_alpha(0.96)
    for whisker, implementation in zip(box["whiskers"], [name for name in implementations for _ in range(2)]):
        whisker.set_color(COLORS[implementation])
    for cap, implementation in zip(box["caps"], [name for name in implementations for _ in range(2)]):
        cap.set_color(COLORS[implementation])

    ax.set_title(
        "\n".join(
            [
                "CMS Insertion Throughput (5 \u00d7 32768)",
                f"Data: {next(iter(total_items)):,} Zipf-distributed int64 values (s=1.1, support=100k)",
                f"Sketch fixed to 5 x {next(iter(cols))}; 10 seeded runs",
            ]
        )
    )
    ax.set_ylabel("Throughput (items/sec)")
    ax.set_xticks(positions)
    ax.set_xticklabels([LABELS[name] for name in implementations])
    ax.grid(True, axis="y", alpha=0.25, zorder=1)

    max_height = max(max(values) for values in grouped.values())
    ax.set_ylim(0, max_height * 1.18)
    for bar, value in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{value / 1_000_000:.2f}M",
            ha="center",
            va="bottom",
            fontsize=11,
        )

    fig.tight_layout()
    fig.savefig(output, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    render_plot(load_rows(args.input), args.output)
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
