#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import json
import os
import statistics
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

IMPL_SKETCHLIB = "asap_sketchlib"
IMPL_POLARS = "polars (exact)"

COLORS = {
    "insert": "#4C78A8",
    "sort": "#F58518",
    "query": "#54A24B",
}


def load_csv(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise ValueError(f"{path} contains no data rows")
    return rows


def infer_total_items(raw_report: Path | None, fallback: int) -> int:
    if raw_report is None or not raw_report.exists():
        return fallback
    with raw_report.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            report = json.loads(line)
            size = report.get("workload", {}).get("size")
            if size:
                return int(size)
    return fallback


def infer_query_count(raw_report: Path | None, fallback: int) -> int:
    if raw_report is None or not raw_report.exists():
        return fallback
    with raw_report.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            report = json.loads(line)
            accuracy = report.get("bench", {}).get("accuracy", {})
            grid_points = accuracy.get("grid_points")
            if grid_points:
                return int(grid_points)
    return fallback


def medians_by_impl(rows: list[dict[str, str]], value_column: str) -> dict[str, float]:
    grouped: dict[str, list[float]] = {}
    for row in rows:
        grouped.setdefault(row["impl"], []).append(float(row[value_column]))
    return {impl: statistics.median(values) for impl, values in grouped.items()}


def render_plot(
    insert_rows: list[dict[str, str]],
    finalize_rows: list[dict[str, str]],
    query_rows: list[dict[str, str]],
    output: Path,
    total_items: int,
    query_count: int,
) -> None:
    insert_tput = medians_by_impl(insert_rows, "throughput_items_per_sec")
    finalize_ns = medians_by_impl(finalize_rows, "finalize_nanoseconds")
    query_tput = medians_by_impl(query_rows, "throughput_queries_per_sec")

    def insert_ms(impl: str) -> float:
        return (total_items / insert_tput[impl]) * 1_000.0

    def query_ms(impl: str) -> float:
        return (query_count / query_tput[impl]) * 1_000.0

    series = [
        {
            "label": "asap_sketchlib KLL",
            "insert": insert_ms(IMPL_SKETCHLIB),
            "sort": 0.0,
            "query": query_ms(IMPL_SKETCHLIB),
        },
        {
            "label": "Polars",
            "insert": insert_ms(IMPL_POLARS),
            "sort": finalize_ns[IMPL_POLARS] / 1_000_000.0,
            "query": query_ms(IMPL_POLARS),
        },
    ]

    output.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 4.8))
    y_positions = list(range(len(series)))
    left = [0.0 for _ in series]
    for key, name in [
        ("insert", "Insert"),
        ("sort", "Sort + quantile grid"),
        ("query", "Query"),
    ]:
        values = [row[key] for row in series]
        ax.barh(
            y_positions,
            values,
            left=left,
            height=0.48,
            color=COLORS[key],
            label=name,
            alpha=0.9,
        )
        left = [l + v for l, v in zip(left, values)]

    totals = [row["insert"] + row["sort"] + row["query"] for row in series]
    max_total = max(totals)
    for y, row, total in zip(y_positions, series, totals):
        ax.text(
            total + max_total * 0.02,
            y,
            f"{total:,.1f} ms",
            va="center",
            ha="left",
            fontsize=11,
            color="#222222",
        )
        for key in ("insert", "sort", "query"):
            value = row[key]
            if value <= 0 or value < max_total * 0.045:
                continue
            start = row["insert"] if key == "sort" else row["insert"] + row["sort"] if key == "query" else 0.0
            ax.text(
                start + value / 2.0,
                y,
                f"{value:,.1f}",
                va="center",
                ha="center",
                fontsize=10,
                color="white",
                weight="bold",
            )

    ax.set_title(
        "\n".join(
            [
                "KLL Quantile Workflow Latency",
                f"Median of 10 runs; {total_items:,} Zipf-distributed int64 values; {query_count} quantiles",
            ]
        )
    )
    ax.set_xlabel("Time (ms)")
    ax.set_yticks(y_positions)
    ax.set_yticklabels([row["label"] for row in series])
    ax.invert_yaxis()
    ax.grid(True, axis="x", alpha=0.25)
    ax.legend(loc="upper right", frameon=False)
    ax.set_xlim(0, max_total * 1.2)

    fig.tight_layout()
    fig.savefig(output, dpi=200)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--insert", required=True, type=Path)
    parser.add_argument("--finalize", required=True, type=Path)
    parser.add_argument("--query", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--raw-report", type=Path)
    parser.add_argument("--total-items", type=int, default=10_000_000)
    parser.add_argument("--query-count", type=int, default=101)
    args = parser.parse_args()

    total_items = infer_total_items(args.raw_report, args.total_items)
    query_count = infer_query_count(args.raw_report, args.query_count)
    render_plot(
        load_csv(args.insert),
        load_csv(args.finalize),
        load_csv(args.query),
        args.output,
        total_items,
        query_count,
    )
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
