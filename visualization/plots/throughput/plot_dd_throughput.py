#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
import statistics
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

COLORS = {
    "rust_sketchlib_dd": "#4C78A8",
    "polars_quantile": "#B279A2",
}

LABELS = {
    "rust_sketchlib_dd": "Rust sketchlib",
    "polars_quantile": "Polars quantile",
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def render_plot(rows: list[dict[str, str]], output: Path) -> None:
    grouped: dict[str, list[float]] = {}
    for row in rows:
        grouped.setdefault(row["implementation"], []).append(float(row["throughput_items_per_sec"]))
    total_items = {int(row["total_items"]) for row in rows}
    alpha_values = {float(row["alpha"]) for row in rows}
    implementations = [name for name in ("rust_sketchlib_dd", "polars_quantile") if name in grouped]
    if not implementations:
        raise ValueError("no known implementations present in input")
    medians = [statistics.median(grouped[name]) for name in implementations]
    max_height = max(max(values) for values in grouped.values())

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(8.2, 5.6))
    positions = list(range(len(implementations)))
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
        whiskerprops={"linewidth": 1.4},
        capprops={"linewidth": 1.4},
        boxprops={"linewidth": 1.2},
    )
    for patch, name in zip(box["boxes"], implementations):
        patch.set_facecolor("#ffffff")
        patch.set_edgecolor(COLORS[name])
        patch.set_alpha(0.96)
    for whisker, name in zip(box["whiskers"], [name for name in implementations for _ in range(2)]):
        whisker.set_color(COLORS[name])
    for cap, name in zip(box["caps"], [name for name in implementations for _ in range(2)]):
        cap.set_color(COLORS[name])

    ax.set_title(
        "\n".join(
            [
                "DD Sketch Insertion Throughput",
                f"Data: {next(iter(total_items)):,} Zipf-distributed int64 values (s=1.1, support=100k)",
                f"alpha={next(iter(alpha_values)):.4f}; 10 runs",
            ]
        )
    )
    ax.set_ylabel("Throughput (items/sec)")
    ax.set_xticks(positions)
    ax.set_xticklabels([LABELS[name] for name in implementations])
    ax.grid(True, axis="y", alpha=0.25, zorder=1)
    ax.set_ylim(0, max_height * 1.18)
    for bar, median in zip(bars, medians):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            f"{median / 1_000_000:.2f}M",
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
