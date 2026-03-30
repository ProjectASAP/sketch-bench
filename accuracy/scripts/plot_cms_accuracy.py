#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
from collections import defaultdict
from matplotlib.lines import Line2D
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

COLORS = {
    "rust_datasketches_cms": "#54A24B",
    "rust_sketchlib_cms": "#4C78A8",
    "rust_sketchlib_cs": "#4C78A8",
    "cpp_datasketches_cms": "#F58518",
}
IMPLEMENTATION_LABELS = {
    "rust_datasketches_cms": "Rust DataSketches",
    "rust_sketchlib_cms": "Rust sketchlib",
    "rust_sketchlib_cs": "Rust sketchlib",
    "cpp_datasketches_cms": "C++ DataSketches",
}
ALL_COLUMNS = [2048, 4096, 8192, 16384, 32768, 65536, 131072]
VARIANT_CONFIG = {
    "cms": {
        "implementations": [
            "rust_datasketches_cms",
            "rust_sketchlib_cms",
            "cpp_datasketches_cms",
        ],
        "grouped_offsets": {
            "rust_datasketches_cms": -0.24,
            "rust_sketchlib_cms": 0.0,
            "cpp_datasketches_cms": 0.24,
        },
        "title": "CMS Accuracy",
    },
    "cs": {
        "implementations": ["rust_sketchlib_cs", "cpp_datasketches_cms"],
        "grouped_offsets": {
            "rust_sketchlib_cs": -0.12,
            "cpp_datasketches_cms": 0.12,
        },
        "title": "CS Accuracy",
    },
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        rows = list(reader)
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def parse_dataset_spec(spec: str) -> tuple[str, Path]:
    label, separator, raw_path = spec.partition("=")
    if not separator or not label or not raw_path:
        raise ValueError(f"invalid dataset spec {spec!r}; expected LABEL=PATH")
    return label, Path(raw_path)


def group_rows_by_implementation_and_cols(
    rows: list[dict[str, str]],
) -> dict[str, dict[int, list[float]]]:
    grouped: dict[str, dict[int, list[float]]] = defaultdict(lambda: defaultdict(list))
    for row in rows:
        grouped[row["implementation"]][int(row["cols"])].append(
            float(row["median_relative_error"]) * 100.0
        )
    return grouped


def require_column_data(
    grouped: dict[str, dict[int, list[float]]],
    implementations: list[str],
    x_values: list[int],
    output_path: Path,
) -> None:
    for implementation in implementations:
        for col in x_values:
            if not grouped[implementation][col]:
                raise ValueError(
                    f"missing data for implementation={implementation}, cols={col} while writing {output_path}"
                )


def base_title_lines(title_prefix: str) -> list[str]:
    return [
        f"{title_prefix}: Heavy Hitters Only Median-Over-Seeds Relative Error",
        "Data: 10M Zipf-distributed int64 values (s=1.1, support=100k)",
        "Heavy hitters only (true_count >= 100)",
        "Sketch fixed to 5 rows",
    ]


def plot_grouped_boxplots(
    grouped: dict[str, dict[int, list[float]]],
    implementations: list[str],
    grouped_offsets: dict[str, float],
    output_path: Path,
    x_values: list[int],
    title_lines: list[str],
) -> None:
    require_column_data(grouped, implementations, x_values, output_path)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(12.5, 5.5))
    width = 0.18

    for implementation in implementations:
        positions = [index + grouped_offsets[implementation] for index in range(len(x_values))]
        data = [grouped[implementation][col] for col in x_values]
        bp = ax.boxplot(
            data,
            positions=positions,
            widths=width,
            patch_artist=True,
            showfliers=False,
            medianprops={"color": "#222222", "linewidth": 2},
            whiskerprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
            capprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
            boxprops={"facecolor": COLORS.get(implementation, "#333333"), "edgecolor": COLORS.get(implementation, "#333333"), "alpha": 0.75},
        )
        for patch in bp["boxes"]:
            patch.set_facecolor(COLORS.get(implementation, "#333333"))
            patch.set_edgecolor(COLORS.get(implementation, "#333333"))
            patch.set_alpha(0.75)

        medians = []
        for values in data:
            sorted_values = sorted(values)
            medians.append(sorted_values[len(sorted_values) // 2])
        ax.plot(
            positions,
            medians,
            color=COLORS.get(implementation, "#333333"),
            linewidth=2,
            marker="o",
            markersize=5,
            zorder=3,
        )

    ax.set_title("\n".join(title_lines))
    ax.set_xlabel("Columns")
    ax.set_ylabel("Median-Over-Seeds Relative Error (%)")
    ax.set_xticks(range(len(x_values)))
    ax.set_xticklabels([str(value) for value in x_values])
    ax.grid(True, axis="y", alpha=0.25)
    legend_handles = [
        Line2D([0], [0], color=COLORS[name], lw=6, label=name) for name in implementations
    ]
    ax.legend(handles=legend_handles)
    fig.tight_layout()
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def plot_single_column_boxplot(
    grouped: dict[str, dict[int, list[float]]],
    implementations: list[str],
    output_path: Path,
    col: int,
    title_lines: list[str],
) -> None:
    require_column_data(grouped, implementations, [col], output_path)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(8.5, 5.5))
    positions = list(range(len(implementations)))
    data = [grouped[implementation][col] for implementation in implementations]
    bp = ax.boxplot(
        data,
        positions=positions,
        widths=0.55,
        patch_artist=True,
        showfliers=False,
        medianprops={"color": "#222222", "linewidth": 2},
        whiskerprops={"linewidth": 1.5},
        capprops={"linewidth": 1.5},
        boxprops={"alpha": 0.75},
    )

    for patch, implementation in zip(bp["boxes"], implementations):
        color = COLORS.get(implementation, "#333333")
        patch.set_facecolor(color)
        patch.set_edgecolor(color)
        patch.set_alpha(0.75)

    for whisker, implementation in zip(bp["whiskers"], [name for name in implementations for _ in range(2)]):
        whisker.set_color(COLORS.get(implementation, "#333333"))
    for cap, implementation in zip(bp["caps"], [name for name in implementations for _ in range(2)]):
        cap.set_color(COLORS.get(implementation, "#333333"))

    ax.set_title("\n".join(title_lines))
    ax.set_xlabel("Implementation")
    ax.set_ylabel("Median-Over-Seeds Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels(implementations, rotation=12, ha="right")
    ax.grid(True, axis="y", alpha=0.25)
    fig.tight_layout()
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def write_all_plots(rows: list[dict[str, str]], variant: str, output_path: Path) -> list[Path]:
    variant_config = VARIANT_CONFIG[variant]
    implementations = variant_config["implementations"]
    grouped_offsets = variant_config["grouped_offsets"]
    title_prefix = variant_config["title"]
    grouped = group_rows_by_implementation_and_cols(rows)
    written_paths = [output_path]
    stem = output_path.stem
    suffix = output_path.suffix

    plot_grouped_boxplots(
        grouped,
        implementations,
        grouped_offsets,
        output_path,
        ALL_COLUMNS,
        base_title_lines(title_prefix),
    )

    from_8192_path = output_path.with_name(f"{stem}_from_8192{suffix}")
    plot_grouped_boxplots(
        grouped,
        implementations,
        grouped_offsets,
        from_8192_path,
        [8192, 16384, 32768, 65536, 131072],
        base_title_lines(title_prefix) + ["Columns shown: 8192 and above"],
    )
    written_paths.append(from_8192_path)

    from_16384_path = output_path.with_name(f"{stem}_from_16384{suffix}")
    plot_grouped_boxplots(
        grouped,
        implementations,
        grouped_offsets,
        from_16384_path,
        [16384, 32768, 65536, 131072],
        base_title_lines(title_prefix) + ["Columns shown: 16384 and above"],
    )
    written_paths.append(from_16384_path)

    for col in ALL_COLUMNS:
        per_col_path = output_path.with_name(f"{stem}_col_{col}{suffix}")
        plot_single_column_boxplot(
            grouped,
            implementations,
            per_col_path,
            col,
            [
                f"{title_prefix}: Heavy Hitters Only Median-Over-Seeds Relative Error",
                f"Column count: {col}",
                "Data: 10M Zipf-distributed int64 values (s=1.1, support=100k)",
                "Heavy hitters only (true_count >= 100)",
            ],
        )
        written_paths.append(per_col_path)

    return written_paths


def group_rows_for_dataset_seed_boxplot(
    dataset_rows: list[tuple[str, list[dict[str, str]]]],
    implementations: list[str],
    cols: int,
    seed: int,
) -> dict[str, dict[str, list[float]]]:
    grouped: dict[str, dict[str, list[float]]] = {}
    for dataset_label, rows in dataset_rows:
        implementation_data: dict[str, list[float]] = defaultdict(list)
        for row in rows:
            row_cols = int(row["cols"])
            row_seed = int(row["seed"])
            if row_cols != cols or row_seed != seed:
                continue
            implementation_data[row["implementation"]].append(float(row["relative_error"]) * 100.0)
        grouped[dataset_label] = implementation_data

    for dataset_label, implementation_data in grouped.items():
        for implementation in implementations:
            if not implementation_data.get(implementation):
                raise ValueError(
                    f"missing data for dataset={dataset_label}, implementation={implementation}, "
                    f"cols={cols}, seed={seed}"
                )
    return grouped


def plot_dataset_seed_boxplot(
    dataset_rows: list[tuple[str, list[dict[str, str]]]],
    variant: str,
    output_path: Path,
    cols: int,
    seed: int,
) -> None:
    variant_config = VARIANT_CONFIG[variant]
    implementations = variant_config["implementations"]
    title_prefix = variant_config["title"]
    grouped = group_rows_for_dataset_seed_boxplot(dataset_rows, implementations, cols, seed)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(11.5, 5.8))
    width = 0.65
    gap = 1.4
    positions: list[float] = []
    tick_labels: list[str] = []
    dataset_centers: list[tuple[float, str]] = []

    cursor = 0.0
    for dataset_label, _ in dataset_rows:
        dataset_positions = []
        for implementation in implementations:
            position = cursor
            dataset_positions.append(position)
            positions.append(position)
            tick_labels.append(IMPLEMENTATION_LABELS.get(implementation, implementation))
            cursor += 1.0
        dataset_centers.append(
            ((dataset_positions[0] + dataset_positions[-1]) / 2.0, dataset_label.upper())
        )
        cursor += gap

    box_index = 0
    for dataset_label, _ in dataset_rows:
        for implementation in implementations:
            values = grouped[dataset_label][implementation]
            bp = ax.boxplot(
                [values],
                positions=[positions[box_index]],
                widths=width,
                patch_artist=True,
                showfliers=False,
                medianprops={"color": "#222222", "linewidth": 2},
                whiskerprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
                capprops={"color": COLORS.get(implementation, "#333333"), "linewidth": 1.5},
                boxprops={
                    "facecolor": COLORS.get(implementation, "#333333"),
                    "edgecolor": COLORS.get(implementation, "#333333"),
                    "alpha": 0.78,
                },
            )
            for patch in bp["boxes"]:
                patch.set_facecolor(COLORS.get(implementation, "#333333"))
                patch.set_edgecolor(COLORS.get(implementation, "#333333"))
                patch.set_alpha(0.78)
            box_index += 1

    ax.set_title(
        "\n".join(
            [
                f"{title_prefix}: Relative Error by Dataset at Fixed Columns and Seed",
                f"Columns: {cols}; hash seed: {seed}",
                "Heavy hitters only (true_count >= 100)",
            ]
        )
    )
    ax.set_ylabel("Relative Error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels(tick_labels, rotation=15, ha="right")
    ax.grid(True, axis="y", alpha=0.25)

    for center, dataset_label in dataset_centers:
        ax.text(center, -0.22, dataset_label, transform=ax.get_xaxis_transform(), ha="center")

    if len(dataset_centers) > 1:
        for index in range(len(dataset_centers) - 1):
            separator_x = positions[(index + 1) * len(implementations)] - (gap + 1.0) / 2.0
            ax.axvline(separator_x, color="#bbbbbb", linewidth=1.0, linestyle="--", alpha=0.7)

    legend_handles = [
        Line2D([0], [0], color=COLORS[name], lw=6, label=IMPLEMENTATION_LABELS.get(name, name))
        for name in implementations
    ]
    ax.legend(handles=legend_handles, loc="upper right")
    fig.tight_layout()
    fig.subplots_adjust(bottom=0.24)
    fig.savefig(output_path, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--variant", required=True, choices=sorted(VARIANT_CONFIG))
    parser.add_argument("--input-key-errors", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--dataset-key-seed-errors",
        action="append",
        default=[],
        help="Dataset-labeled seed-level key error CSV in the form LABEL=PATH; repeat per dataset",
    )
    parser.add_argument("--dataset-boxplot-output", type=Path)
    parser.add_argument("--dataset-boxplot-cols", type=int)
    parser.add_argument("--dataset-boxplot-seed", type=int)
    args = parser.parse_args()

    rows = load_rows(args.input_key_errors)
    for path in write_all_plots(rows, args.variant, args.output):
        print(f"Wrote {path}")

    if args.dataset_key_seed_errors or args.dataset_boxplot_output:
        if len(args.dataset_key_seed_errors) < 2:
            raise ValueError("dataset comparison plot requires at least two --dataset-key-seed-errors inputs")
        if args.dataset_boxplot_output is None:
            raise ValueError("dataset comparison plot requires --dataset-boxplot-output")
        if args.dataset_boxplot_cols is None:
            raise ValueError("dataset comparison plot requires --dataset-boxplot-cols")
        if args.dataset_boxplot_seed is None:
            raise ValueError("dataset comparison plot requires --dataset-boxplot-seed")

        dataset_rows = [
            (label, load_rows(path))
            for label, path in (parse_dataset_spec(spec) for spec in args.dataset_key_seed_errors)
        ]
        plot_dataset_seed_boxplot(
            dataset_rows,
            args.variant,
            args.dataset_boxplot_output,
            args.dataset_boxplot_cols,
            args.dataset_boxplot_seed,
        )
        print(f"Wrote {args.dataset_boxplot_output}")


if __name__ == "__main__":
    main()
