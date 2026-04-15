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

COLOR = "#4C78A8"


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def group_rows(
    rows: list[dict[str, str]],
) -> tuple[dict[float, list[float]], list[float]]:
    grouped: dict[float, list[float]] = defaultdict(list)
    alpha_values: set[float] = set()
    for row in rows:
        alpha = float(row["alpha"])
        grouped[alpha].append(float(row["relative_error"]) * 100.0)
        alpha_values.add(alpha)
    return grouped, sorted(alpha_values)


def format_alpha(alpha: float) -> str:
    return f"{alpha * 100:.1f}%"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    rows = load_rows(args.input)
    grouped, alpha_values = group_rows(rows)
    for alpha in alpha_values:
        if not grouped[alpha]:
            raise ValueError(f"missing rows for alpha={alpha}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 5.8))

    positions = list(range(len(alpha_values)))
    data = [grouped[alpha] for alpha in alpha_values]
    box = ax.boxplot(
        data,
        positions=positions,
        widths=0.55,
        patch_artist=True,
        showfliers=False,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"color": COLOR, "linewidth": 1.5},
        capprops={"color": COLOR, "linewidth": 1.5},
        boxprops={"facecolor": COLOR, "edgecolor": COLOR, "alpha": 0.78},
    )
    for patch in box["boxes"]:
        patch.set_facecolor(COLOR)
        patch.set_edgecolor(COLOR)
        patch.set_alpha(0.78)

    medians = []
    for values in data:
        sorted_values = sorted(values)
        medians.append(sorted_values[len(sorted_values) // 2])
    ax.plot(
        positions,
        medians,
        color=COLOR,
        linewidth=2,
        marker="o",
        markersize=5,
        zorder=3,
    )

    ax.set_title(
        "\n".join(
            [
                "DDSketch Accuracy: Quantile Relative Error (p0-p100)",
                "Data: 10M Zipf-distributed int64 values (s=1.1, support=100k)",
                "101 quantile queries per alpha setting",
            ]
        )
    )
    ax.set_xlabel("alpha (relative accuracy)")
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels([format_alpha(alpha) for alpha in alpha_values])
    ax.grid(True, axis="y", alpha=0.25)
    fig.tight_layout()
    fig.savefig(args.output, dpi=200)
    plt.close(fig)


if __name__ == "__main__":
    main()
