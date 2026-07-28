#!/usr/bin/env python3
"""4-algorithm insert-throughput comparison (cms / countsketch / hll / kll).

Reads JSONL records emitted by aqpbm-cli (`source: cli`) and cpp-bench
(`source: cpp-bench`) and renders one subplot per algorithm. Each bar is
one `(language, impl)` pair; height is `bench.throughput_items_per_sec.mean`.

Usage:
    python plot_algorithm_compare.py --input-dir DIR --output PATH
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
from collections import defaultdict
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

ALGORITHMS = [
    ("cms", "Count-Min Sketch"),
    ("countsketch", "Count Sketch"),
    ("hll", "HyperLogLog"),
    ("kll", "KLL Quantile"),
]

LANG_COLOR = {
    "rust": "#4C78A8",
    "cpp": "#E45756",
}

EXCLUDE_IMPLS = {"exact", "polars"}


def load_records(input_dir: Path) -> list[dict]:
    out = []
    for path in sorted(input_dir.glob("*.jsonl")):
        with path.open() as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                out.append(json.loads(line))
    return out


def algorithm_of(rec: dict) -> str:
    return rec["sketch"]


def label_of(rec: dict) -> str:
    lang = rec.get("language", "?")
    impl = rec.get("impl", "?")
    return f"{lang}/{impl}"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input-dir", required=True, type=Path)
    ap.add_argument("--output", required=True, type=Path)
    ap.add_argument("--include-baselines", action="store_true",
                    help="Include exact / polars baselines (default: drop)")
    args = ap.parse_args()

    records = load_records(args.input_dir)

    # group: algorithm -> label -> [throughput_mean per record]
    grouped: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for rec in records:
        algo = algorithm_of(rec)
        if not args.include_baselines and rec.get("impl") in EXCLUDE_IMPLS:
            continue
        bench = rec.get("bench", {})
        tput = bench.get("throughput_items_per_sec", {}).get("mean")
        if tput is None:
            continue
        grouped[algo][label_of(rec)].append(float(tput))

    fig, axes = plt.subplots(2, 2, figsize=(16, 11))
    axes = axes.flatten()

    for ax, (algo, title) in zip(axes, ALGORITHMS):
        data = grouped.get(algo, {})
        if not data:
            ax.set_title(f"{title} — no data")
            ax.axis("off")
            continue
        # Order: rust first (by descending median), then cpp (descending).
        rust_labels = sorted(
            [l for l in data if l.startswith("rust/")],
            key=lambda l: -statistics.median(data[l]),
        )
        cpp_labels = sorted(
            [l for l in data if l.startswith("cpp/")],
            key=lambda l: -statistics.median(data[l]),
        )
        labels = rust_labels + cpp_labels
        medians = [statistics.median(data[l]) / 1e6 for l in labels]
        colors = [LANG_COLOR["rust"] if l.startswith("rust/") else LANG_COLOR["cpp"]
                  for l in labels]

        pos = list(range(len(labels)))
        bars = ax.bar(pos, medians, color=colors, alpha=0.88, width=0.7)
        ax.set_xticks(pos)
        ax.set_xticklabels([l.split("/", 1)[1] for l in labels],
                           rotation=35, ha="right", fontsize=9)
        ax.set_ylabel("Throughput (M items/sec)")
        ax.set_title(f"{title} ({algo})", fontsize=13)
        ax.grid(True, axis="y", alpha=0.25, zorder=0)
        ymax = max(medians) if medians else 1.0
        ax.set_ylim(0, ymax * 1.18)
        for bar, v in zip(bars, medians):
            ax.text(bar.get_x() + bar.get_width() / 2,
                    bar.get_height() + ymax * 0.015,
                    f"{v:.1f}M", ha="center", va="bottom", fontsize=9)

    # Legend on the figure.
    handles = [plt.Rectangle((0, 0), 1, 1, color=LANG_COLOR["rust"], alpha=0.88),
               plt.Rectangle((0, 0), 1, 1, color=LANG_COLOR["cpp"], alpha=0.88)]
    fig.legend(handles, ["Rust", "C++"], loc="upper right",
               bbox_to_anchor=(0.99, 0.99), fontsize=11)

    fig.suptitle(
        "Insert throughput — Zipf(s=1.1) workload, 1M items, support=100k\n"
        "median across measurement runs, sketch params: hll lg_k=14, kll k=200, cms/cs 5×2048",
        fontsize=14,
    )
    fig.tight_layout(rect=(0, 0, 1, 0.93))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=180)
    plt.close(fig)
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
