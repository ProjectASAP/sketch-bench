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
from matplotlib.lines import Line2D

COLORS = {
    "rust_sketchlib_kll": "#4C78A8",
    "rust_oxide_kll": "#54A24B",
    "cpp_datasketches_kll": "#F58518",
}
IMPLEMENTATIONS = [
    "rust_sketchlib_kll",
    "rust_oxide_kll",
    "cpp_datasketches_kll",
]
OFFSETS = {
    "rust_sketchlib_kll": -0.24,
    "rust_oxide_kll": 0.0,
    "cpp_datasketches_kll": 0.24,
}
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
) -> tuple[dict[str, dict[int, list[float]]], list[int]]:
    grouped: dict[str, dict[int, list[float]]] = defaultdict(lambda: defaultdict(list))
    k_values: set[int] = set()
    for row in rows:
        k = int(row["k"])
        grouped[row["implementation"]][k].append(
            float(row["relative_error"]) * 100.0
        )
        k_values.add(k)
    return grouped, sorted(k_values)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    rows = load_rows(args.input)
    grouped, k_values = group_rows(rows)
    for impl_name in IMPLEMENTATIONS:
        for k in k_values:
            if not grouped[impl_name][k]:
                raise ValueError(
                    f"missing rows for implementation={impl_name}, k={k}"
                )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(12.5, 5.5))
    width = 0.18

    for impl_name in IMPLEMENTATIONS:
        positions = [
            idx + OFFSETS[impl_name] for idx in range(len(k_values))
        ]
        data = [grouped[impl_name][k] for k in k_values]
        bp = ax.boxplot(
            data,
            positions=positions,
            widths=width,
            patch_artist=True,
            showfliers=False,
            medianprops={"color": "#222222", "linewidth": 2},
            whiskerprops={"color": COLORS[impl_name], "linewidth": 1.5},
            capprops={"color": COLORS[impl_name], "linewidth": 1.5},
            boxprops={
                "facecolor": COLORS[impl_name],
                "edgecolor": COLORS[impl_name],
                "alpha": 0.78,
            },
        )
        for patch in bp["boxes"]:
            patch.set_facecolor(COLORS[impl_name])
            patch.set_edgecolor(COLORS[impl_name])
            patch.set_alpha(0.78)

        medians = []
        for values in data:
            sorted_values = sorted(values)
            medians.append(sorted_values[len(sorted_values) // 2])
        ax.plot(
            positions,
            medians,
            color=COLORS[impl_name],
            linewidth=2,
            marker="o",
            markersize=5,
            zorder=3,
        )

    ax.set_title(
        "\n".join(
            [
                "KLL Accuracy: Quantile Relative Error (p0-p100)",
                DATASET_LABEL,
                "101 quantile queries per (implementation, k) pair",
            ]
        )
    )
    ax.set_xlabel("k (accuracy parameter)")
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks(range(len(k_values)))
    ax.set_xticklabels([str(k) for k in k_values])
    ax.grid(True, axis="y", alpha=0.25)
    ax.legend(
        handles=[
            Line2D([0], [0], color=COLORS[name], lw=6, label=name)
            for name in IMPLEMENTATIONS
        ]
    )
    fig.tight_layout()
    fig.savefig(args.output, dpi=200)
    plt.close(fig)


if __name__ == "__main__":
    main()
