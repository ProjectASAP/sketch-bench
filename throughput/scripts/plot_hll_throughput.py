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
    "rust_sketchlib_hll": "#4C78A8",
    "rust_sketchlib_hll_hip": "#72B7B2",
    "rust_oxide_hll": "#54A24B",
    "rust_datasketches_hll": "#E45756",
    "cpp_datasketches_hll": "#F58518",
}

LABELS = {
    "rust_sketchlib_hll": "Rust sketchlib\n(ErtlMLE)",
    "rust_sketchlib_hll_hip": "Rust sketchlib\n(HIP)",
    "rust_oxide_hll": "Rust sketch_oxide",
    "rust_datasketches_hll": "Rust DataSketches",
    "cpp_datasketches_hll": "C++ DataSketches",
}

IMPLEMENTATIONS = [
    "rust_sketchlib_hll",
    "rust_sketchlib_hll_hip",
    "rust_oxide_hll",
    "rust_datasketches_hll",
    "cpp_datasketches_hll",
]


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(rows: list[dict[str, str]], output: Path) -> None:
    grouped: dict[str, list[float]] = defaultdict(list)
    total_items = set()
    lg_k_values = set()
    registers_values = set()
    for row in rows:
        grouped[row["implementation"]].append(float(row["throughput_items_per_sec"]))
        total_items.add(int(row["total_items"]))
        lg_k_values.add(int(row["lg_k"]))
        registers_values.add(int(row["registers"]))

    for impl_name in IMPLEMENTATIONS:
        if not grouped.get(impl_name):
            raise ValueError(f"missing data for {impl_name}")

    medians = [statistics.median(grouped[name]) for name in IMPLEMENTATIONS]
    positions = list(range(len(IMPLEMENTATIONS)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 5.8))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=[COLORS[name] for name in IMPLEMENTATIONS],
        alpha=0.88,
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[name] for name in IMPLEMENTATIONS],
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
    for patch, impl_name in zip(box["boxes"], IMPLEMENTATIONS):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(COLORS[impl_name])
        patch.set_alpha(0.96)
    for whisker, impl_name in zip(
        box["whiskers"],
        [name for name in IMPLEMENTATIONS for _ in range(2)],
    ):
        whisker.set_color(COLORS[impl_name])
    for cap, impl_name in zip(
        box["caps"],
        [name for name in IMPLEMENTATIONS for _ in range(2)],
    ):
        cap.set_color(COLORS[impl_name])

    lg_k = next(iter(lg_k_values))
    regs = next(iter(registers_values))
    ax.set_title(
        "\n".join(
            [
                "HLL Insertion Throughput",
                f"Data: {next(iter(total_items)):,} Zipf-distributed int64 values (s=1.1, support=100k)",
                f"lg_k={lg_k} ({regs:,} registers); 10 runs per implementation",
            ]
        )
    )
    ax.set_ylabel("Throughput (items/sec)")
    ax.set_xticks(positions)
    ax.set_xticklabels([LABELS[name] for name in IMPLEMENTATIONS])
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
