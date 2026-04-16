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

ROOT = Path(__file__).resolve().parent.parent

COLORS = {
    "rust_sketchlib_cms": "#4C78A8",
    "rust_oxide_cms": "#54A24B",
    "rust_datasketches_cms": "#E45756",
    "cpp_datasketches_cms": "#F58518",
    "rust_sketchlib_cs": "#4C78A8",
    "rust_oxide_cs": "#54A24B",
    "cpp_insert_optimized_cs": "#F58518",
    "rust_sketchlib_hll": "#4C78A8",
    "rust_sketchlib_hll_hip": "#72B7B2",
    "rust_oxide_hll": "#54A24B",
    "rust_datasketches_hll": "#E45756",
    "cpp_datasketches_hll": "#F58518",
    "rust_sketchlib_kll": "#4C78A8",
    "rust_oxide_kll": "#54A24B",
    "cpp_datasketches_kll": "#F58518",
    "cpp_insert_optimized_kll": "#E45756",
    "rust_sketchlib_dd": "#4C78A8",
    "polars_freq": "#B279A2",
    "polars_cardinality": "#B279A2",
    "polars_quantile": "#B279A2",
}

LABELS = {
    "rust_sketchlib_cms": "CMS sketchlib",
    "rust_oxide_cms": "CMS oxide",
    "rust_datasketches_cms": "CMS rust DS",
    "cpp_datasketches_cms": "CMS C++ DS",
    "rust_sketchlib_cs": "CS sketchlib",
    "rust_oxide_cs": "CS oxide",
    "cpp_insert_optimized_cs": "CS AWS",
    "rust_sketchlib_hll": "HLL sketchlib",
    "rust_sketchlib_hll_hip": "HLL HIP",
    "rust_oxide_hll": "HLL oxide",
    "rust_datasketches_hll": "HLL rust DS",
    "cpp_datasketches_hll": "HLL C++ DS",
    "rust_sketchlib_kll": "KLL sketchlib",
    "rust_oxide_kll": "KLL oxide",
    "cpp_datasketches_kll": "KLL C++ DS",
    "cpp_insert_optimized_kll": "KLL AWS",
    "rust_sketchlib_dd": "DD sketchlib",
    "polars_freq": "Polars freq",
    "polars_cardinality": "Polars cardinality",
    "polars_quantile": "Polars quantile",
}

PANELS = {
    "freq": {
        "title": "Frequency",
        "implementations": [
            "rust_sketchlib_cms",
            "rust_oxide_cms",
            "rust_datasketches_cms",
            "cpp_datasketches_cms",
            "rust_sketchlib_cs",
            "rust_oxide_cs",
            "cpp_insert_optimized_cs",
            "polars_freq",
        ],
    },
    "cardinality": {
        "title": "Cardinality",
        "implementations": [
            "rust_sketchlib_hll",
            "rust_sketchlib_hll_hip",
            "rust_oxide_hll",
            "rust_datasketches_hll",
            "cpp_datasketches_hll",
            "polars_cardinality",
        ],
    },
    "quantile": {
        "title": "Quantile",
        "implementations": [
            "rust_sketchlib_kll",
            "rust_oxide_kll",
            "cpp_datasketches_kll",
            "cpp_insert_optimized_kll",
            "rust_sketchlib_dd",
            "polars_quantile",
        ],
    },
}


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no rows")
    return rows


def throughput_samples(op: str, family: str, implementation: str, rows: list[dict[str, str]]) -> list[float]:
    if op == "insert":
        return [float(row["throughput_items_per_sec"]) for row in rows if row["implementation"] == implementation]

    matching = [row for row in rows if row["implementation"] == implementation]
    if family == "freq":
        return [float(row["throughput_queries_per_sec"]) for row in matching]
    if family == "cardinality":
        return [(1e9 / float(row["nanoseconds"])) if float(row["nanoseconds"]) > 0 else 0.0 for row in matching]
    if family == "quantile" and implementation == "polars_quantile":
        batches: dict[tuple[int, int], dict[str, float]] = {}
        for row in matching:
            key = (int(row["run"]), int(row["repeat"]))
            batch = batches.setdefault(key, {"nanoseconds": float(row["nanoseconds"]), "count": 0.0})
            batch["count"] += 1.0
        return [
            (batch["count"] * 1e9 / batch["nanoseconds"]) if batch["nanoseconds"] > 0 else 0.0
            for batch in batches.values()
        ]
    return [(1e9 / float(row["nanoseconds"])) if float(row["nanoseconds"]) > 0 else 0.0 for row in matching]


def sources_for(op: str) -> dict[str, list[tuple[Path, list[str]]]]:
    suffix = "throughput_results.csv" if op == "insert" else "throughput_query_results.csv"
    freq_polars = ROOT / "polars_freq" / "output" / ("polars_freq_throughput_results.csv" if op == "insert" else "polars_freq_query_results.csv")
    cardinality_polars = ROOT / "polars_cardinality" / "output" / ("polars_cardinality_throughput_results.csv" if op == "insert" else "polars_cardinality_query_results.csv")
    quantile_polars = ROOT / "polars_quantile" / "output" / ("polars_quantile_throughput_results.csv" if op == "insert" else "polars_quantile_query_results.csv")
    return {
        "freq": [
            (ROOT / "cms" / "output" / f"cms_{suffix}", ["rust_sketchlib_cms", "rust_oxide_cms", "rust_datasketches_cms", "cpp_datasketches_cms"]),
            (ROOT / "cs" / "output" / f"cs_{suffix}", ["rust_sketchlib_cs", "rust_oxide_cs", "cpp_insert_optimized_cs"]),
            (freq_polars, ["polars_freq"]),
        ],
        "cardinality": [
            (ROOT / "hll" / "output" / f"hll_{suffix}", ["rust_sketchlib_hll", "rust_sketchlib_hll_hip", "rust_oxide_hll", "rust_datasketches_hll", "cpp_datasketches_hll"]),
            (cardinality_polars, ["polars_cardinality"]),
        ],
        "quantile": [
            (ROOT / "kll" / "output" / f"kll_{suffix}", ["rust_sketchlib_kll", "rust_oxide_kll", "cpp_datasketches_kll", "cpp_insert_optimized_kll"]),
            (ROOT / "dd" / "output" / f"dd_{suffix}", ["rust_sketchlib_dd"]),
            (quantile_polars, ["polars_quantile"]),
        ],
    }


def render(op: str, output: Path) -> None:
    fig, axes = plt.subplots(1, 3, figsize=(19, 6.5))
    panel_sources = sources_for(op)
    ylabel = "Throughput (items/sec)" if op == "insert" else "Throughput (queries/sec)"

    for ax, family in zip(axes, ("freq", "cardinality", "quantile")):
        grouped: dict[str, list[float]] = {}
        for path, implementations in panel_sources[family]:
            rows = load_rows(path)
            for implementation in implementations:
                samples = throughput_samples(op, family, implementation, rows)
                if samples:
                    grouped[implementation] = samples

        implementations = [name for name in PANELS[family]["implementations"] if name in grouped]
        if not implementations:
            raise ValueError(f"no data found for {family}")

        medians = [statistics.median(grouped[name]) for name in implementations]
        positions = list(range(len(implementations)))
        bars = ax.bar(
            positions,
            medians,
            width=0.58,
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
        for patch, implementation in zip(box["boxes"], implementations):
            patch.set_facecolor("#ffffff")
            patch.set_edgecolor(COLORS[implementation])
            patch.set_alpha(0.96)

        max_height = max(max(values) for values in grouped.values())
        ax.set_ylim(0, max_height * 1.2)
        ax.set_title(PANELS[family]["title"])
        ax.set_ylabel(ylabel)
        ax.set_xticks(positions)
        ax.set_xticklabels([LABELS[name] for name in implementations], rotation=22, ha="right")
        ax.grid(True, axis="y", alpha=0.25, zorder=1)
        for bar, value in zip(bars, medians):
            label = f"{value / 1_000_000:.2f}M" if value >= 1_000_000 else f"{value:,.0f}"
            ax.text(
                bar.get_x() + bar.get_width() / 2,
                bar.get_height() + max_height * 0.02,
                label,
                ha="center",
                va="bottom",
                fontsize=10,
            )

    title = "Throughput Overview: Insertion" if op == "insert" else "Throughput Overview: Query"
    subtitle = "Polars uses lazy end-to-end batch execution; quantile uses one batched p0..p100 collect"
    fig.suptitle(f"{title}\n{subtitle}", fontsize=16)
    fig.tight_layout(rect=(0, 0, 1, 0.93))
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--op", required=True, choices=("insert", "query"))
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    render(args.op, args.output)
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
