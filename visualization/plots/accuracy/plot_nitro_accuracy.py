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

COLOR = "#E45756"
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


def group_rows(
    rows: list[dict[str, str]],
) -> tuple[dict[float, list[float]], list[float], int]:
    grouped: dict[float, list[float]] = defaultdict(list)
    cols = set()
    rate_values: set[float] = set()
    for row in rows:
        rate = float(row["rate"])
        grouped[rate].append(float(row["avg_relative_error"]) * 100.0)
        rate_values.add(rate)
        cols.add(int(row["cols"]))
    if len(cols) != 1:
        raise ValueError("nitro plot expects a single fixed cols setting")
    return grouped, sorted(rate_values, reverse=True), next(iter(cols))


def format_rate(rate: float) -> str:
    return f"{rate * 100:.0f}%"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    rows = load_rows(args.input)
    grouped, rate_values, cols = group_rows(rows)
    for rate in rate_values:
        if not grouped[rate]:
            raise ValueError(f"missing rows for rate={rate}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(11.0, 5.8))

    positions = list(range(len(rate_values)))
    data = [grouped[rate] for rate in rate_values]
    box = ax.boxplot(
        data,
        positions=positions,
        widths=0.55,
        patch_artist=True,
        showfliers=False,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"color": COLOR, "linewidth": 1.5},
        capprops={"color": COLOR, "linewidth": 1.5},
        boxprops={"facecolor": COLOR, "edgecolor": COLOR, "alpha": 0.8},
    )
    for patch in box["boxes"]:
        patch.set_facecolor(COLOR)
        patch.set_edgecolor(COLOR)
        patch.set_alpha(0.8)

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
                "Nitro Accuracy: Heavy-Hitter Avg Relative Error",
                DATASET_LABEL,
                f"Nitro Count-Min target fixed to 5 x {cols:,}; 10 trials per rate",
            ]
        )
    )
    ax.set_xlabel("Nitro sampling rate")
    ax.set_ylabel("Avg Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels([format_rate(rate) for rate in rate_values])
    ax.grid(True, axis="y", alpha=0.25)
    fig.tight_layout()
    fig.savefig(args.output, dpi=200)
    plt.close(fig)


if __name__ == "__main__":
    main()
