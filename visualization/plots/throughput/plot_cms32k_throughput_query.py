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

TITLE = "CMS32K Query Throughput"
COLS_DEFAULT = 32768


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(rows: list[dict[str, str]], output: Path, title: str, cols_default: int) -> None:
    grouped: dict[str, list[float]] = defaultdict(list)
    cols = set()
    total_items = set()
    total_queries = set()
    for row in rows:
        grouped[row["implementation"]].append(float(row["throughput_queries_per_sec"]))
        cols.add(int(row["cols"]))
        total_items.add(int(row["total_items"]))
        total_queries.add(int(row["total_queries"]))

    implementations = [name for name in COLORS if grouped.get(name)]
    if not implementations:
        raise ValueError("no known implementations present in input")
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
        zorder=2,
    )

    box = ax.boxplot(
        [grouped[name] for name in implementations],
        positions=positions,
        widths=0.22,
        patch_artist=True,
        showfliers=False,
        zorder=3,
        medianprops={"color": "#222222", "linewidth": 2},
    )
    for patch, name in zip(box["boxes"], implementations):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(COLORS[name])

    cols_str = next(iter(cols)) if cols else cols_default
    q_str = next(iter(total_queries)) if total_queries else "?"
    n_str = next(iter(total_items)) if total_items else "?"
    ax.set_title(
        "\n".join([
            title,
            f"Data: {n_str:,} items; {q_str:,} distinct-key queries per seed",
            f"Sketch fixed to 5 x {cols_str}; 10 seeded runs",
        ])
    )
    ax.set_ylabel("Throughput (queries/sec)")
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

    render_plot(load_rows(args.input), args.output, TITLE, COLS_DEFAULT)
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
