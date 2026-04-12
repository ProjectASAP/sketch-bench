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
    "rust_oxide_hll": "#54A24B",
    "rust_datasketches_hll": "#E45756",
    "cpp_datasketches_hll": "#F58518",
}

LABELS = {
    "rust_sketchlib_hll": "Rust sketchlib",
    "rust_oxide_hll": "Rust sketch_oxide",
    "rust_datasketches_hll": "Rust DataSketches",
    "cpp_datasketches_hll": "C++ DataSketches",
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(rows: list[dict[str, str]], output: Path) -> None:
    grouped: dict[str, list[float]] = defaultdict(list)
    total_items = set()
    lg_k = set()
    for row in rows:
        grouped[row["implementation"]].append(float(row["nanoseconds"]))
        total_items.add(int(row["total_items"]))
        lg_k.add(int(row["lg_k"]))

    implementations = [name for name in COLORS if grouped.get(name)]
    if not implementations:
        raise ValueError("no known implementations present")
    medians = [statistics.median(grouped[name]) for name in implementations]
    positions = list(range(len(implementations)))

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 5.8))
    bars = ax.bar(
        positions,
        medians,
        width=0.55,
        color=[COLORS[name] for name in implementations],
        alpha=0.88,
    )
    ax.boxplot(
        [grouped[name] for name in implementations],
        positions=positions,
        widths=0.22,
        patch_artist=True,
        showfliers=False,
        medianprops={"color": "#222", "linewidth": 2},
    )

    n_str = next(iter(total_items)) if total_items else "?"
    k_str = next(iter(lg_k)) if lg_k else "?"
    ax.set_title(
        "\n".join([
            "HLL get_estimate() Latency",
            f"Data: {n_str:,} int64 values; lg_k={k_str}",
            "10 runs x 10 calls per run",
        ])
    )
    ax.set_ylabel("Nanoseconds per get_estimate() call (median)")
    ax.set_xticks(positions)
    ax.set_xticklabels([LABELS[name] for name in implementations])
    ax.grid(True, axis="y", alpha=0.25)

    max_height = max(max(values) for values in grouped.values())
    ax.set_ylim(0, max_height * 1.18)
    for bar, value in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{value:.0f} ns",
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
