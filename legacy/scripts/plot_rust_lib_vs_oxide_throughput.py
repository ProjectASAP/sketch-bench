#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

ITEM_COUNT = 1_000_000
LIB_COLOR = "#4C78A8"
OXIDE_COLOR = "#F58518"
TITLE_FONT_SIZE = 20
AXIS_LABEL_FONT_SIZE = 18
TICK_LABEL_FONT_SIZE = 17
VALUE_LABEL_FONT_SIZE = 18
BAR_WIDTH = 0.26


@dataclass(frozen=True)
class BenchmarkSeries:
    label: str
    input_path: Path
    samples_ns: list[float]


@dataclass(frozen=True)
class PlotSpec:
    title: str
    series_specs: list[tuple[str, Path]]
    output_path: Path


def load_jsonl_times(path: Path) -> list[float]:
    if not path.is_file():
        raise FileNotFoundError(f"Expected input file does not exist: {path}")

    samples: list[float] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_number, raw_line in enumerate(handle, start=1):
            line = raw_line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {exc.msg}") from exc

            if "total_nanoseconds" not in record:
                raise ValueError(f"{path}:{line_number}: missing total_nanoseconds")

            value = record["total_nanoseconds"]
            if not isinstance(value, (int, float)):
                raise ValueError(
                    f"{path}:{line_number}: total_nanoseconds must be numeric, got {type(value).__name__}"
                )

            numeric_value = float(value)
            if not math.isfinite(numeric_value):
                raise ValueError(f"{path}:{line_number}: total_nanoseconds must be finite")
            if numeric_value <= 0:
                raise ValueError(f"{path}:{line_number}: total_nanoseconds must be positive")

            samples.append(numeric_value)

    if not samples:
        raise ValueError(f"{path}: file contains zero valid benchmark rows")

    return samples


def median_throughput_from_ns(samples: list[float], item_count: int = ITEM_COUNT) -> float:
    median_total_nanoseconds = statistics.median(samples)
    if median_total_nanoseconds <= 0:
        raise ValueError("Median total_nanoseconds must be positive")
    return item_count * 1_000_000_000 / median_total_nanoseconds


def build_plot_specs(rust_output_dir: Path, plots_dir: Path) -> list[PlotSpec]:
    repo_root = rust_output_dir.parent.parent
    pairs = [
        (
            "Rust CountMin\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "countmin_lib_fixedmatrix_fast.jsonl"),
                ("oxide", rust_output_dir / "countmin_oxide.jsonl"),
            ],
            "rust_countmin_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust CountSketch\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "countsketch_lib_fixedmatrix_fast.jsonl"),
                ("oxide", rust_output_dir / "countsketch_oxide.jsonl"),
            ],
            "rust_countsketch_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust Elastic\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "elastic_lib.jsonl"),
                ("oxide", rust_output_dir / "elastic_oxide.jsonl"),
            ],
            "rust_elastic_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust HLL\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "hll_lib.jsonl"),
                ("oxide", rust_output_dir / "hll_oxide.jsonl"),
            ],
            "rust_hll_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust KLL\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "kll_lib.jsonl"),
                ("oxide", rust_output_dir / "kll_oxide.jsonl"),
            ],
            "rust_kll_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust Nitro\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "nitro_lib.jsonl"),
                ("oxide", rust_output_dir / "nitro_oxide.jsonl"),
            ],
            "rust_nitro_lib_vs_oxide_throughput.png",
        ),
        (
            "Rust UnivMon\nThroughput Comparison",
            [
                ("sketchlib", rust_output_dir / "univmon_lib.jsonl"),
                ("oxide", rust_output_dir / "univmon_oxide.jsonl"),
            ],
            "rust_univmon_lib_vs_oxide_throughput.png",
        ),
        (
            "CPP DataSketches vs Rust Sketchlib\nCountSketch Throughput",
            [
                ("cpp datasketches", repo_root / "cpp/output/cs_datasketches.jsonl"),
                ("rust sketchlib", rust_output_dir / "countsketch_lib_fixedmatrix_fast.jsonl"),
            ],
            "cpp_datasketches_cs_vs_rust_sketchlib_cs_throughput.png",
        ),
        (
            "CPP DataSketches vs Rust Sketchlib\nKLL Throughput",
            [
                ("cpp datasketches", repo_root / "cpp/output/kll_datasketches.jsonl"),
                ("rust sketchlib", rust_output_dir / "kll_lib.jsonl"),
            ],
            "cpp_datasketches_kll_vs_rust_sketchlib_kll_throughput.png",
        ),
        (
            "Rust CountMin\nThroughput Comparison",
            [
                ("datasketches", rust_output_dir / "countmin_datasketches.jsonl"),
                ("sketchlib", rust_output_dir / "countmin_lib_fixedmatrix_fast.jsonl"),
                ("oxide", rust_output_dir / "countmin_oxide.jsonl"),
            ],
            "rust_datasketches_sketchlib_oxide_countmin_throughput.png",
        ),
        (
            "Rust HLL\nThroughput Comparison",
            [
                ("datasketches", rust_output_dir / "hll_datasketches.jsonl"),
                ("sketchlib", rust_output_dir / "hll_lib.jsonl"),
                ("oxide", rust_output_dir / "hll_oxide.jsonl"),
            ],
            "rust_datasketches_sketchlib_oxide_hll_throughput.png",
        ),
    ]

    return [
        PlotSpec(
            title=title,
            series_specs=series_specs,
            output_path=plots_dir / output_name,
        )
        for title, series_specs, output_name in pairs
    ]


def format_throughput_label(value: float) -> str:
    if value >= 1_000_000_000:
        return f"{value / 1_000_000_000:.2f}B"
    if value >= 1_000_000:
        return f"{value / 1_000_000:.2f}M"
    if value >= 1_000:
        return f"{value / 1_000:.2f}K"
    return f"{value:.0f}"


def render_comparison_plot(spec: PlotSpec) -> None:
    series = [
        BenchmarkSeries(label=label, input_path=path, samples_ns=load_jsonl_times(path))
        for label, path in spec.series_specs
    ]
    throughputs = [median_throughput_from_ns(item.samples_ns) for item in series]

    spec.output_path.parent.mkdir(parents=True, exist_ok=True)

    fig_width = 6.6 if len(series) == 2 else 7.2
    fig, ax = plt.subplots(figsize=(fig_width, 4.4))
    x_spacing = 0.34 if len(series) == 2 else 0.30
    x_positions = [index * x_spacing for index in range(len(series))]
    colors = {
        "datasketches": "#54A24B",
        "cpp datasketches": "#54A24B",
        "sketchlib": LIB_COLOR,
        "rust sketchlib": LIB_COLOR,
        "oxide": OXIDE_COLOR,
    }
    bars = ax.bar(
        x_positions,
        throughputs,
        color=[colors[item.label] for item in series],
        width=BAR_WIDTH,
    )

    ax.set_ylabel("Throughput (items/sec)", fontsize=AXIS_LABEL_FONT_SIZE)
    ax.set_title(
        spec.title,
        fontsize=TITLE_FONT_SIZE,
        pad=14,
    )
    ax.tick_params(axis="both", labelsize=TICK_LABEL_FONT_SIZE)
    ax.set_xticks(x_positions, [item.label for item in series])
    ax.set_xlim(-0.24, x_positions[-1] + 0.24)
    ax.set_axisbelow(True)
    ax.grid(axis="y", linestyle="--", alpha=0.3)

    max_height = max(throughputs)
    ax.set_ylim(0, max_height * 1.15 if max_height > 0 else 1)

    for bar, value in zip(bars, throughputs):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + max_height * 0.02,
            format_throughput_label(value),
            ha="center",
            va="bottom",
            fontsize=VALUE_LABEL_FONT_SIZE,
        )

    fig.tight_layout()
    fig.savefig(spec.output_path, dpi=200)
    plt.close(fig)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate Rust sketchlib-vs-oxide throughput comparison plots."
    )
    parser.add_argument(
        "--rust-output-dir",
        type=Path,
        default=Path("rust/output"),
        help="Directory containing Rust benchmark JSONL files.",
    )
    parser.add_argument(
        "--plots-dir",
        type=Path,
        default=Path("plots"),
        help="Directory where PNG plots will be written.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        specs = build_plot_specs(args.rust_output_dir, args.plots_dir)
        for spec in specs:
            render_comparison_plot(spec)
            print(f"Wrote {spec.output_path}")
    except Exception as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
