#!/usr/bin/env python3
"""Configurable accuracy bar chart with error bars.

Reads one or more `accuracy.jsonl` reports (emitted by
`sketchlib bench --accuracy --report ...`) and draws a bar chart where
the x-axis is `sketch type x dimension` and the y-axis is an accuracy
metric. Bars are grouped/coloured by implementation; error bars are
configurable.

The error metric is NOT comparable across statistic groups — HLL is a
cardinality relative error, CMS/CountSketch is a frequency relative
error, KLL/DD is a quantile rank error. Use `--layout facet` to keep
each group on its own y-axis, or `--layout combined` (the single shared
axis you asked for) and read across families with that caveat in mind.

Error-bar sources (see `--error`):
  * If several input records share a (family, impl, dimension) key —
    e.g. you ran the bench across multiple `--seed`s and appended to the
    same JSONL — the spread is computed across those records (real CI /
    stddev / IQR).
  * Otherwise, with `--error p99`, the within-record upper percentile
    (`relative_error_p99` / `max_rank_err`) is drawn as a one-sided
    whisker where the comparator provides it.

Examples:
  # Single combined axis, mean +/- 95% CI, all families in the report
  plot_accuracy_bars.py --input output/accuracy/accuracy.jsonl \
      --output visualization/plots/accuracy/accuracy_bars.png

  # Just HLL + CMS, median +/- IQR, log y
  plot_accuracy_bars.py --input output/accuracy/*.jsonl \
      --families hll,cms --bar-stat median --error iqr --log \
      --output /tmp/acc.png

  # Force KLL to be plotted as a relative error instead of rank error
  plot_accuracy_bars.py --input out.jsonl --families kll \
      --metric mean_relative_err --output /tmp/kll.png
"""
from __future__ import annotations

import argparse
import json
import math
import os
from collections import defaultdict
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D

# ----------------------------------------------------------------------
# Family metadata. Kept in sync with sketch-cli/src/raw_csv.rs (dimension
# param columns) and sketch-bench/src/accuracy/*.rs (comparator json keys).
# ----------------------------------------------------------------------
FAMILY_GROUP = {
    "hll": "cardinality",
    "cms": "frequency",
    "countsketch": "frequency",
    "cs": "frequency",
    "elastic": "frequency",
    "kll": "quantile",
    "dd": "quantile",
    "ddsketch": "quantile",
}

# Canonical accuracy metric per statistic group, used when --metric auto.
GROUP_DEFAULT_METRIC = {
    "cardinality": "relative_error",
    "frequency": "relative_error_mean",
    "quantile": "mean_rank_err",
    "topk": "recall_at_k",
}

# Within-record upper-percentile companion for --error p99.
METRIC_P99_PARTNER = {
    "relative_error_mean": "relative_error_p99",
    "mean_rank_err": "max_rank_err",
}

# Default dimension param key per family (the first is preferred).
FAMILY_DIM_KEYS = {
    "hll": ["registers", "lg_k"],
    "cms": ["cols", "rows"],
    "countsketch": ["cols", "rows"],
    "cs": ["cols", "rows"],
    "elastic": ["buckets", "depth"],
    "kll": ["k"],
    "dd": ["alpha"],
    "ddsketch": ["alpha"],
}

# Stable colours for the implementations we see most; anything else gets
# a colour from the tab20 cycle.
IMPL_COLORS = {
    "oxide": "#4C78A8",
    "ertlmle": "#B279A2",
    "hip": "#72B7B2",
    "datasketches": "#54A24B",
    "exact": "#9D755D",
    "polars": "#E45756",
}
_FALLBACK_CYCLE = list(plt.get_cmap("tab20").colors)


def impl_color(impl: str, assigned: dict[str, str]) -> str:
    if impl in IMPL_COLORS:
        return IMPL_COLORS[impl]
    if impl not in assigned:
        assigned[impl] = matplotlib.colors.to_hex(
            _FALLBACK_CYCLE[len(assigned) % len(_FALLBACK_CYCLE)]
        )
    return assigned[impl]


def family_of(rec: dict) -> str:
    cfg = rec.get("sketch_config") or {}
    return cfg.get("family") or rec.get("sketch") or "?"


def dimension_label(rec: dict, dim_key: str | None) -> str | None:
    """Return the dimension value (as a string) for this record."""
    cfg = rec.get("sketch_config") or {}
    params = cfg.get("params") or {}
    fam = family_of(rec)
    keys = [dim_key] if dim_key else FAMILY_DIM_KEYS.get(fam, list(params.keys()))
    for k in keys:
        if k and k in params:
            return str(params[k])
    return None


def accuracy_section(rec: dict) -> dict | None:
    bench = rec.get("bench") or {}
    acc = bench.get("accuracy")
    return acc if isinstance(acc, dict) else None


def pick_metric(fam: str, requested: str) -> str:
    if requested != "auto":
        return requested
    group = FAMILY_GROUP.get(fam, "frequency")
    return GROUP_DEFAULT_METRIC.get(group, "relative_error_mean")


# ----------------------------------------------------------------------
# Aggregation helpers
# ----------------------------------------------------------------------
def mean(xs: list[float]) -> float:
    return sum(xs) / len(xs)


def stddev(xs: list[float]) -> float:
    if len(xs) < 2:
        return 0.0
    m = mean(xs)
    return math.sqrt(sum((x - m) ** 2 for x in xs) / (len(xs) - 1))


def median(xs: list[float]) -> float:
    s = sorted(xs)
    n = len(s)
    mid = n // 2
    return s[mid] if n % 2 else 0.5 * (s[mid - 1] + s[mid])


def quantile(xs: list[float], q: float) -> float:
    s = sorted(xs)
    if len(s) == 1:
        return s[0]
    pos = q * (len(s) - 1)
    lo = int(math.floor(pos))
    hi = min(lo + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (pos - lo)


def bar_and_error(
    samples: list[float],
    p99_samples: list[float],
    bar_stat: str,
    error: str,
    scale: float,
) -> tuple[float, float, float]:
    """Return (height, err_lo, err_hi) already scaled."""
    vals = [v * scale for v in samples]
    height = median(vals) if bar_stat == "median" else mean(vals)

    if error == "none" or not vals:
        return height, 0.0, 0.0
    if error == "stddev":
        s = stddev(vals)
        return height, s, s
    if error == "ci95":
        if len(vals) < 2:
            return height, 0.0, 0.0
        half = 1.96 * stddev(vals) / math.sqrt(len(vals))
        return height, half, half
    if error == "iqr":
        lo = quantile(vals, 0.25)
        hi = quantile(vals, 0.75)
        return height, max(0.0, height - lo), max(0.0, hi - height)
    if error == "p99":
        # One-sided whisker to the within-record upper percentile.
        if p99_samples:
            top = max(p99_samples) * scale
            return height, 0.0, max(0.0, top - height)
        return height, 0.0, 0.0
    return height, 0.0, 0.0


# ----------------------------------------------------------------------
# Loading
# ----------------------------------------------------------------------
def load_records(paths: list[Path]) -> list[dict]:
    records: list[dict] = []
    for p in paths:
        with p.open("r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                records.append(json.loads(line))
    if not records:
        raise SystemExit(f"no records found in {', '.join(map(str, paths))}")
    return records


def build_groups(
    records: list[dict],
    families: set[str] | None,
    impls: set[str] | None,
    metric_arg: str,
    dim_key: str | None,
):
    """Collapse records into {(family, dim): {impl: ([samples],[p99])}}."""
    groups: dict[tuple, dict[str, tuple[list, list]]] = defaultdict(
        lambda: defaultdict(lambda: ([], []))
    )
    metric_by_group: dict[str, str] = {}
    skipped = 0
    for rec in records:
        fam = family_of(rec)
        if families and fam not in families:
            continue
        impl = rec.get("impl") or "?"
        if impls and impl not in impls:
            continue
        acc = accuracy_section(rec)
        if not acc:
            skipped += 1
            continue
        metric = pick_metric(fam, metric_arg)
        metric_by_group[FAMILY_GROUP.get(fam, "other")] = metric
        if metric not in acc:
            skipped += 1
            continue
        dim = dimension_label(rec, dim_key)
        if dim is None:
            continue
        samples, p99 = groups[(fam, dim)][impl]
        samples.append(float(acc[metric]))
        partner = METRIC_P99_PARTNER.get(metric)
        if partner and partner in acc:
            p99.append(float(acc[partner]))
    if not groups:
        raise SystemExit(
            "no records carried the requested accuracy metric "
            "(run with --accuracy, or pick --metric to match the family)"
        )
    return groups, skipped


# ----------------------------------------------------------------------
# Plotting
# ----------------------------------------------------------------------
def dim_sort_key(dim: str):
    try:
        return (0, float(dim))
    except ValueError:
        return (1, dim)


def draw_axis(ax, groups, args, impl_order, assigned_colors):
    scale = 100.0 if args.percent else 1.0
    # Ordered x positions: sort by (family, numeric dimension).
    group_keys = sorted(groups.keys(), key=lambda k: (k[0], dim_sort_key(k[1])))
    n_impl = len(impl_order)
    bar_w = 0.8 / max(1, n_impl)

    xticks, xlabels = [], []
    for gi, (fam, dim) in enumerate(group_keys):
        xticks.append(gi)
        xlabels.append(f"{fam}\n{dim}")
        by_impl = groups[(fam, dim)]
        for ii, impl in enumerate(impl_order):
            if impl not in by_impl:
                continue
            samples, p99 = by_impl[impl]
            height, lo, hi = bar_and_error(
                samples, p99, args.bar_stat, args.error, scale
            )
            x = gi + (ii - (n_impl - 1) / 2.0) * bar_w
            ax.bar(
                x,
                height,
                width=bar_w * 0.92,
                color=impl_color(impl, assigned_colors),
                edgecolor="#222222",
                linewidth=0.4,
                zorder=2,
            )
            if lo or hi:
                ax.errorbar(
                    x,
                    height,
                    yerr=[[lo], [hi]],
                    fmt="none",
                    ecolor="#222222",
                    elinewidth=1.0,
                    capsize=3,
                    zorder=3,
                )
    ax.set_xticks(xticks)
    ax.set_xticklabels(xlabels)
    ax.grid(True, axis="y", alpha=0.25)
    if args.log:
        ax.set_yscale("log")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--input",
        type=Path,
        nargs="+",
        required=True,
        help="one or more accuracy JSONL reports",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--families",
        default="",
        help="comma list to include (default: all present, e.g. hll,cms,kll)",
    )
    parser.add_argument(
        "--impls", default="", help="comma list of implementations to include"
    )
    parser.add_argument(
        "--metric",
        default="auto",
        help="accuracy json key to plot; 'auto' picks the family default "
        "(cardinality->relative_error, frequency->relative_error_mean, "
        "quantile->mean_rank_err)",
    )
    parser.add_argument(
        "--dimension",
        default=None,
        help="param key to use as the x-axis dimension (default: family default)",
    )
    parser.add_argument(
        "--bar-stat", choices=["mean", "median"], default="mean"
    )
    parser.add_argument(
        "--error",
        choices=["ci95", "stddev", "iqr", "p99", "none"],
        default="ci95",
        help="error-bar source (ci95/stddev/iqr across records sharing a "
        "(family,dim,impl) key; p99 = within-record upper whisker)",
    )
    parser.add_argument(
        "--layout",
        choices=["combined", "facet"],
        default="combined",
        help="combined = single shared y-axis; facet = one panel per "
        "statistic group (recommended when mixing families)",
    )
    parser.add_argument(
        "--percent", action="store_true", help="scale metric by 100 (for relative errors)"
    )
    parser.add_argument("--log", action="store_true", help="log-scale y-axis")
    parser.add_argument("--title", default=None)
    args = parser.parse_args()

    families = {f.strip() for f in args.families.split(",") if f.strip()} or None
    impls = {i.strip() for i in args.impls.split(",") if i.strip()} or None

    records = load_records(args.input)
    groups, skipped = build_groups(records, families, impls, args.metric, args.dimension)

    impl_order = sorted({impl for g in groups.values() for impl in g})
    assigned_colors: dict[str, str] = {}
    ylabel = "Relative error (%)" if args.percent else "Accuracy metric"

    args.output.parent.mkdir(parents=True, exist_ok=True)

    if args.layout == "facet":
        # One panel per statistic group present.
        present_groups: dict[str, dict] = defaultdict(dict)
        for (fam, dim), by_impl in groups.items():
            present_groups[FAMILY_GROUP.get(fam, "other")][(fam, dim)] = by_impl
        gnames = sorted(present_groups)
        fig, axes = plt.subplots(
            len(gnames), 1, figsize=(max(10, 1.6 * sum(len(g) for g in present_groups.values())), 4.5 * len(gnames)), squeeze=False
        )
        for ax, gname in zip(axes[:, 0], gnames):
            draw_axis(ax, present_groups[gname], args, impl_order, assigned_colors)
            ax.set_title(f"{gname}  ·  metric={GROUP_DEFAULT_METRIC.get(gname, args.metric) if args.metric=='auto' else args.metric}")
            ax.set_ylabel(ylabel)
        axes[-1, 0].set_xlabel("sketch · dimension")
    else:
        fig, ax = plt.subplots(figsize=(max(10, 1.4 * len(groups)), 6))
        draw_axis(ax, groups, args, impl_order, assigned_colors)
        ax.set_xlabel("sketch · dimension")
        ax.set_ylabel(ylabel)

    title = args.title or (
        f"Accuracy by sketch × dimension  "
        f"(bar={args.bar_stat}, err={args.error})"
    )
    fig.suptitle(title, y=0.995)

    handles = [
        Line2D([0], [0], color=impl_color(impl, assigned_colors), lw=8, label=impl)
        for impl in impl_order
    ]
    fig.legend(
        handles=handles,
        loc="upper center",
        bbox_to_anchor=(0.5, 0.95),
        ncol=min(len(handles), 6),
        frameon=False,
    )

    if skipped:
        fig.text(
            0.01,
            0.005,
            f"note: {skipped} record(s) skipped (no accuracy / metric absent)",
            fontsize=8,
            color="#666666",
        )

    fig.tight_layout(rect=(0, 0, 1, 0.88))
    fig.savefig(args.output, dpi=200)
    plt.close(fig)
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
