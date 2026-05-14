#!/usr/bin/env python3
"""Plot OctoSketch throughput: regular vs Octo 1/2/4/8 threads for CMS, CS, HLL."""
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

BAR_SPECS = [
    ("regular", 1, "Regular (1T)", "#4C78A8"),
    ("octo", 1, "Octo 1T", "#72B7B2"),
    ("octo", 2, "Octo 2T", "#54A24B"),
    ("octo", 4, "Octo 4T", "#F58518"),
    ("octo", 8, "Octo 8T", "#E45756"),
]

SKETCH_TITLES = {
    "cms": "Count-Min Sketch (5 x 32768)",
    "cs": "Count Sketch (5 x 32768)",
    "hll": "HyperLogLog (p=14)",
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(
    all_rows: list[dict[str, str]], sketch_type: str, output: Path
) -> None:
    grouped: dict[tuple[str, int], list[float]] = defaultdict(list)
    total_items = set()
    for row in all_rows:
        if row["sketch_type"] != sketch_type:
            continue
        key = (row["implementation"], int(row["num_workers"]))
        grouped[key].append(float(row["throughput_items_per_sec"]))
        total_items.add(int(row["total_items"]))

    active_specs = [
        (impl_name, nw, label, color)
        for impl_name, nw, label, color in BAR_SPECS
        if grouped.get((impl_name, nw))
    ]
    if not active_specs:
        raise ValueError(f"no data for sketch_type={sketch_type}")

    labels = [label for _, _, label, _ in active_specs]
    colors = [color for _, _, _, color in active_specs]
    keys = [(impl_name, nw) for impl_name, nw, _, _ in active_specs]
    medians = [statistics.median(grouped[k]) for k in keys]
    positions = list(range(len(active_specs)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 5.8))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=colors,
        alpha=0.88,
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[k] for k in keys],
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
    for patch, color in zip(box["boxes"], colors):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(color)
        patch.set_alpha(0.96)
    for whisker, color in zip(
        box["whiskers"], [c for c in colors for _ in range(2)]
    ):
        whisker.set_color(color)
    for cap, color in zip(box["caps"], [c for c in colors for _ in range(2)]):
        cap.set_color(color)

    title_name = SKETCH_TITLES.get(sketch_type, sketch_type.upper())
    ax.set_title(
        "\n".join(
            [
                f"OctoSketch {title_name} Insertion Throughput",
                f"Data: {next(iter(total_items)):,} Zipf-distributed int64 values (s=1.1, support=100k)",
                f"10 runs per configuration",
            ]
        )
    )
    ax.set_ylabel("Throughput (items/sec)")
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=8, ha="right")
    ax.grid(True, axis="y", alpha=0.25, zorder=1)

    max_height = max(max(v) for v in grouped.values())
    ax.set_ylim(0, max_height * 1.18)
    for bar, value in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{value / 1_000_000:.2f}M",
            ha="center",
            va="bottom",
            fontsize=10,
        )

    fig.tight_layout()
    fig.savefig(output, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output-cms", required=True, type=Path)
    parser.add_argument("--output-cs", required=True, type=Path)
    parser.add_argument("--output-hll", required=True, type=Path)
    args = parser.parse_args()

    rows = load_rows(args.input)
    render_plot(rows, "cms", args.output_cms)
    print(f"Wrote {args.output_cms}")
    render_plot(rows, "cs", args.output_cs)
    print(f"Wrote {args.output_cs}")
    render_plot(rows, "hll", args.output_hll)
    print(f"Wrote {args.output_hll}")


if __name__ == "__main__":
    main()
