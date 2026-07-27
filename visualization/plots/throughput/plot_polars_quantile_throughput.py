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

COLOR = "#B279A2"
LABEL = "Polars quantile"
TITLE = "Polars Quantile Insertion Throughput"


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def build_throughput(row: dict[str, str]) -> float:
    """Items per second to produce a *queryable* result.

    Not the CSV's own ``throughput_items_per_sec``: that column is
    ``total_items / total_nanoseconds``, and for a polars row
    ``total_nanoseconds`` covers only the ``Vec::push`` that buffers the
    stream. All of the engine work — DataFrame build, group_by / sort /
    quantile grid, collect — is billed to ``finalize_nanoseconds``, so the
    ready-made column reports the buffer and overstates these rows by orders
    of magnitude. Divide by the sum, which is what the legacy polars binaries
    timed and what this chart's title claims to show.
    """
    total_ns = float(row["total_nanoseconds"]) + float(row.get("finalize_nanoseconds") or 0.0)
    if total_ns <= 0:
        raise ValueError(f"non-positive elapsed time in row: {row}")
    return float(row["total_items"]) * 1_000_000_000.0 / total_ns


def render_plot(rows: list[dict[str, str]], output: Path) -> None:
    values = [build_throughput(row) for row in rows]
    total_items = {int(row["total_items"]) for row in rows}

    median = statistics.median(values)
    output.parent.mkdir(parents=True, exist_ok=True)

    fig, ax = plt.subplots(figsize=(6.8, 5.6))
    bars = ax.bar([0], [median], width=0.52, color=COLOR, alpha=0.9, zorder=2)
    box = ax.boxplot(
        [values],
        positions=[0],
        widths=0.18,
        patch_artist=True,
        showfliers=False,
        zorder=3,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"linewidth": 1.4},
        capprops={"linewidth": 1.4},
        boxprops={"linewidth": 1.2},
    )
    box["boxes"][0].set_facecolor("#ffffff")
    box["boxes"][0].set_edgecolor(COLOR)
    box["boxes"][0].set_alpha(0.96)
    for whisker in box["whiskers"]:
        whisker.set_color(COLOR)
    for cap in box["caps"]:
        cap.set_color(COLOR)

    ax.set_title(
        "\n".join(
            [
                TITLE,
                f"Data: {next(iter(total_items)):,} Zipf-distributed int64 values (s=1.1, support=100k)",
                f"{len(values)} repeated runs",
            ]
        )
    )
    ax.set_ylabel("Throughput (items/sec)")
    ax.set_xticks([0])
    ax.set_xticklabels([LABEL])
    ax.grid(True, axis="y", alpha=0.25, zorder=1)

    max_height = max(values)
    ax.set_ylim(0, max_height * 1.18)
    ax.text(
        bars[0].get_x() + bars[0].get_width() / 2,
        bars[0].get_height() + max_height * 0.02,
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
