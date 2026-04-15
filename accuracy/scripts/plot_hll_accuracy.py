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
    "rust_asap_sketchlib_hll_ertlmle": "#4C78A8",
    "rust_asap_sketchlib_hll_hip": "#B279A2",
    "rust_sketch_oxide_hll": "#E45756",
    "rust_datasketches_hll": "#54A24B",
    "cpp_datasketches_hll": "#F58518",
}
IMPLEMENTATIONS = [
    "rust_asap_sketchlib_hll_ertlmle",
    "rust_asap_sketchlib_hll_hip",
    "rust_sketch_oxide_hll",
    "rust_datasketches_hll",
    "cpp_datasketches_hll",
]
OFFSETS = {
    "rust_asap_sketchlib_hll_ertlmle": -0.32,
    "rust_asap_sketchlib_hll_hip": -0.16,
    "rust_sketch_oxide_hll": 0.0,
    "rust_datasketches_hll": 0.16,
    "cpp_datasketches_hll": 0.32,
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
    register_sizes: set[int] = set()
    for row in rows:
        registers = int(row["registers"])
        grouped[row["implementation"]][registers].append(
            float(row["relative_error"]) * 100.0
        )
        register_sizes.add(registers)
    return grouped, sorted(register_sizes)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    rows = load_rows(args.input)
    grouped, register_sizes = group_rows(rows)
    for impl_name in IMPLEMENTATIONS:
        for reg in register_sizes:
            if not grouped[impl_name][reg]:
                raise ValueError(
                    f"missing rows for implementation={impl_name}, registers={reg}"
                )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(12.5, 5.5))
    width = 0.14

    for impl_name in IMPLEMENTATIONS:
        positions = [
            idx + OFFSETS[impl_name] for idx in range(len(register_sizes))
        ]
        data = [grouped[impl_name][reg] for reg in register_sizes]
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
                "HLL Accuracy: Relative Error Over 10 Seeded Input Remappings",
                DATASET_LABEL,
            ]
        )
    )
    ax.set_xlabel("Registers")
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks(range(len(register_sizes)))
    ax.set_xticklabels([str(r) for r in register_sizes])
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
