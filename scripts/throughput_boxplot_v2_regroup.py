#!/usr/bin/env python3
"""Reorganise the throughput_boxplot_v2 figures into three thematic groups,
reading the existing per-panel long-form CSVs (no benchmarks re-run here).

Groups:
  * `insert/`     — cross-impl insertion throughput, one figure per algorithm.
                    polars is excluded; it belongs in `vs_polars/`.
  * `query/`      — cross-impl query throughput, one figure per algorithm.
                    HLL Classic is dropped (Classic vs HIP story lives in the
                    raw CSV). KLL uses a log y-axis because oxide's quantile()
                    is O(num_levels) per call and runs ~4 decades slower than
                    the cached / amortised alternatives.
  * `vs_polars/`  — asap_sketchlib vs polars(exact) only. One figure per
                    algorithm × {insert, query}. CMS/CS uses a 2×2 sub-grid
                    over the four shape configs.

CMS/CS panels are rendered as 2×2 sub-grids (CMS@2K, CMS@32K, CS@2K, CS@32K)
so the four shape configs are easy to compare without making one figure
ridiculously wide.
"""
from __future__ import annotations

import csv
import json
import statistics
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.ticker as mticker

REPO = Path(__file__).resolve().parents[1]
OUT_DIR = REPO / "visualization" / "output" / "throughput_boxplot_v2"
RAW_CMS_CS_DIR = OUT_DIR / "raw" / "cms_cs"
RAW_CMS_CS_HH1001_DIR = OUT_DIR / "raw" / "cms_cs_hh1001"

INSERT_DIR = OUT_DIR / "insert"
QUERY_DIR = OUT_DIR / "query"
VS_POLARS_DIR = OUT_DIR / "vs_polars"
for d in (INSERT_DIR, QUERY_DIR, VS_POLARS_DIR):
    d.mkdir(parents=True, exist_ok=True)

IMPL_COLORS = {
    "asap_sketchlib":            "#1f77b4",
    "asap_sketchlib (Classic)":  "#1f77b4",
    "asap_sketchlib (HIP)":      "#17becf",
    "oxide":                     "#ff7f0e",
    "datasketches (Rust)":       "#2ca02c",
    "datasketches (C++)":        "#d62728",
    "datasketches (C++ Apache)": "#d62728",
    "InsertOptimized":           "#9467bd",
    "polars (exact)":            "#8c564b",
}

LATENCY_COLORS = {
    "insert": "#4C78A8",
    "finalize": "#F58518",
    "query": "#54A24B",
}

GROUP_TITLES = {
    "CMS@2K": "CMS (5x2048)",
    "CMS@32K": "CMS (5x32768)",
    "CS@2K": "CS (5x2048)",
    "CS@32K": "CS (5x32768)",
}


def load_long(csv_path: Path, value_col: str) -> dict[tuple[str, str], list[float]]:
    out: dict[tuple[str, str], list[float]] = defaultdict(list)
    if not csv_path.exists():
        return {}
    with csv_path.open() as f:
        for r in csv.DictReader(f):
            out[(r["group"], r["impl"])].append(float(r[value_col]))
    return dict(out)


def query_count_from_csv(csv_path: Path, fallback: int) -> int:
    if not csv_path.exists():
        return fallback
    with csv_path.open() as f:
        reader = csv.DictReader(f)
        for row in reader:
            value = row.get("total_queries")
            if value:
                return int(value)
    return fallback


def load_frequency_accuracy(report_path: Path) -> dict | None:
    if not report_path.exists():
        return None
    with report_path.open() as f:
        for line in f:
            rec = json.loads(line)
            acc = (rec.get("bench") or {}).get("accuracy")
            if isinstance(acc, dict) and "relative_error_mean" in acc:
                return acc
    return None


def fmt_mops(v: float, _pos=None) -> str:
    if v >= 1e6:
        return f"{v / 1e6:.0f}M"
    if v >= 1e3:
        return f"{v / 1e3:.0f}k"
    return f"{v:.0f}"


def _bar_with_box(ax, positions, values, colors, *, bar_w=0.46, box_w=0.18) -> None:
    means = [sum(v) / len(v) for v in values]
    ax.bar(positions, means, width=bar_w, color=colors, alpha=0.55,
           edgecolor="black", linewidth=0.6, zorder=1)
    bp = ax.boxplot(values, positions=positions, widths=box_w, showfliers=False,
                    patch_artist=True, zorder=3,
                    medianprops=dict(color="black", linewidth=1.2),
                    whiskerprops=dict(color="black", linewidth=0.8),
                    capprops=dict(color="black", linewidth=0.8),
                    boxprops=dict(linewidth=0.8))
    for patch in bp["boxes"]:
        patch.set_facecolor("white")
        patch.set_alpha(0.95)


def _filter(data: dict, group: str, impls: list[str]) -> tuple[list[str], list[list[float]]]:
    labels, values = [], []
    for impl in impls:
        v = data.get((group, impl))
        if v:
            labels.append(impl)
            values.append(v)
    return labels, values


def _annotate_polars_finalize(ax, labels: list[str], positions: list[float],
                              values: list[list[float]],
                              finalize_ns: dict | None, group: str) -> None:
    """Stamp `+sort/+nunique/+groupby: X` above the polars bar when the
    finalize cost is non-trivial. This is what makes the vs_polars insert
    bars honest: the insert bar measures only `Vec::push`; the polars
    exact-baseline really also pays the finalize cost we annotate here."""
    if not finalize_ns:
        return
    op_by_algorithm = {
        "HLL": "+n_unique",
        "KLL": "+sort",
        "CMS@2K": "+groupby", "CMS@32K": "+groupby",
        "CS@2K": "+groupby",  "CS@32K": "+groupby",
    }
    for lab, p, v in zip(labels, positions, values):
        if lab != "polars (exact)":
            continue
        ns_list = finalize_ns.get((group, lab))
        if not ns_list:
            continue
        mean_ns = sum(ns_list) / len(ns_list)
        if mean_ns < 1e6:  # < 1ms — too cheap to be interesting
            continue
        op = op_by_algorithm.get(group, "+finalize")
        mean_ms = mean_ns / 1e6
        label = (f"{op}\n{mean_ms / 1000:.2f} s" if mean_ms >= 1000
                 else f"{op}\n{mean_ms:.0f} ms")
        bar_top = sum(v) / len(v)
        ax.annotate(label,
                    xy=(p, bar_top), xytext=(0, 8), textcoords="offset points",
                    ha="center", va="bottom", fontsize=8,
                    color="#444444",
                    bbox=dict(boxstyle="round,pad=0.2", facecolor="#fff5cc",
                              edgecolor="#c9b400", linewidth=0.5, alpha=0.95))


def single_panel(data: dict, group: str, impls: list[str], *,
                 title: str, ylabel: str, out_png: Path,
                 log: bool = False, finalize_ns: dict | None = None) -> None:
    labels, values = _filter(data, group, impls)
    if not labels:
        print(f"  skip {out_png.name}: no data for group={group}")
        return
    positions = list(range(1, len(labels) + 1))
    colors = [IMPL_COLORS.get(lab, "#777777") for lab in labels]
    fig, ax = plt.subplots(figsize=(max(8.5, 1.6 * len(labels) + 4.0), 5.0))
    _bar_with_box(ax, positions, values, colors)
    _annotate_polars_finalize(ax, labels, positions, values, finalize_ns, group)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=18, ha="right")
    ax.set_title(title, fontsize=11)
    ax.set_ylabel(ylabel)
    if log:
        ax.set_yscale("log")
        ax.yaxis.set_major_formatter(mticker.FuncFormatter(fmt_mops))
    else:
        ax.yaxis.set_major_formatter(mticker.FuncFormatter(fmt_mops))
    ax.grid(axis="y", linestyle=":", alpha=0.6, which="both" if log else "major")
    fig.tight_layout()
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def latency_composition(
    insert_data: dict[tuple[str, str], list[float]],
    finalize_data: dict[tuple[str, str], list[float]],
    query_data: dict[tuple[str, str], list[float]],
    *,
    group: str,
    sketch_impl: str,
    sketch_label: str,
    polars_label: str,
    finalize_label: str,
    title: str,
    out_png: Path,
    query_count: int,
    total_items: int = 10_000_000,
) -> None:
    keys = [(group, sketch_impl), (group, "polars (exact)")]
    if any(k not in insert_data or k not in query_data for k in keys):
        print(f"  skip {out_png.name}: missing insert/query data for {group}")
        return

    def median_ms_from_tput(data: dict, key: tuple[str, str], count: int) -> float:
        return count / statistics.median(data[key]) * 1_000.0

    series = [
        {
            "label": sketch_label,
            "insert": median_ms_from_tput(insert_data, keys[0], total_items),
            "finalize": 0.0,
            "query": median_ms_from_tput(query_data, keys[0], query_count),
        },
        {
            "label": polars_label,
            "insert": median_ms_from_tput(insert_data, keys[1], total_items),
            "finalize": statistics.median(finalize_data.get(keys[1], [0.0])) / 1_000_000.0,
            "query": median_ms_from_tput(query_data, keys[1], query_count),
        },
    ]

    out_png.parent.mkdir(parents=True, exist_ok=True)
    fig, ax = plt.subplots(figsize=(10.5, 4.8))
    y_positions = list(range(len(series)))
    left = [0.0 for _ in series]
    for key, label in [
        ("insert", "Insert"),
        ("finalize", finalize_label),
        ("query", "Query"),
    ]:
        values = [row[key] for row in series]
        ax.barh(
            y_positions,
            values,
            left=left,
            height=0.48,
            color=LATENCY_COLORS[key],
            label=label,
            alpha=0.9,
        )
        left = [l + v for l, v in zip(left, values)]

    totals = [row["insert"] + row["finalize"] + row["query"] for row in series]
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
        start = 0.0
        for key in ("insert", "finalize", "query"):
            value = row[key]
            if value > 0 and value >= max_total * 0.045:
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
            start += value

    ax.set_title(
        f"{title}\nMedian of 10 runs; {total_items:,} Zipf-distributed int64 values; {query_count:,} queries"
    )
    ax.set_xlabel("Time (ms)")
    ax.set_yticks(y_positions)
    ax.set_yticklabels([row["label"] for row in series])
    ax.invert_yaxis()
    ax.grid(True, axis="x", alpha=0.25)
    ax.legend(loc="upper right", frameon=False)
    ax.set_xlim(0, max_total * 1.2)
    fig.tight_layout()
    fig.savefig(out_png, dpi=200)
    plt.close(fig)
    print(f"  wrote {out_png}")


def cms_cs_32k_accuracy(out_png: Path) -> None:
    specs = [
        (
            "CMS (5x32768)",
            RAW_CMS_CS_HH1001_DIR / "cms__lib_fixedmatrix_fast_32k__rows5_cols32768" / "report.jsonl",
        ),
        (
            "CS (5x32768)",
            RAW_CMS_CS_HH1001_DIR / "countsketch__lib_fixedmatrix_fast_32k__rows5_cols32768" / "report.jsonl",
        ),
    ]
    rows = []
    for label, path in specs:
        acc = load_frequency_accuracy(path)
        if acc:
            rows.append((label, acc))
    if not rows:
        print(f"  skip {out_png.name}: no 32K accuracy data")
        return

    labels = [label for label, _acc in rows]
    means = [acc["relative_error_mean"] * 100.0 for _label, acc in rows]
    p99s = [acc.get("relative_error_p99", acc["relative_error_mean"]) * 100.0 for _label, acc in rows]
    upper = [max(0.0, p99 - mean) for mean, p99 in zip(means, p99s)]
    colors = [IMPL_COLORS["asap_sketchlib"]] * len(labels)

    fig, ax = plt.subplots(figsize=(8.2, 5.0))
    positions = list(range(len(labels)))
    ax.bar(positions, means, color=colors, alpha=0.65, edgecolor="black", linewidth=0.6)
    ax.errorbar(
        positions,
        means,
        yerr=[[0.0] * len(labels), upper],
        fmt="none",
        ecolor="#222222",
        elinewidth=1.1,
        capsize=4,
        zorder=3,
    )
    ax.axhline(0, color="#444444", linewidth=0.8)
    ax.set_title(
        "CMS / CountSketch accuracy @ 5x32768\n"
        "heavy hitters: true_count > 1000; 697 keys; 72.59% of stream\n"
        "bar = mean relative error; whisker = p99"
    )
    ax.set_ylabel("Relative error (%)")
    ax.set_xticks(positions)
    ax.set_xticklabels(labels)
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    fig.tight_layout()
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def cms_cs_grid(data: dict, impls: list[str], *,
                groups: list[str],
                title: str, ylabel: str, out_png: Path,
                log: bool = False, finalize_ns: dict | None = None,
                large_text: bool = False) -> None:
    # Sync y-limits across subplots for honest comparison.
    all_vals = []
    panels = []
    for g in groups:
        labs, vals = _filter(data, g, impls)
        panels.append((g, labs, vals))
        for v in vals:
            all_vals.extend(v)
    if not all_vals:
        print(f"  skip {out_png.name}: no data")
        return
    if log:
        positive = [v for v in all_vals if v > 0]
        ymin = max(1.0, min(positive) * 0.6) if positive else 1.0
        ymax = max(all_vals) * 1.35
    else:
        ymin = 0.0
        ymax = max(all_vals) * 1.12

    max_slots = max(len(labs) for _grp, labs, _vals in panels)

    if len(groups) <= 2:
        nrows, ncols = 1, len(groups)
        figsize = (6.8 * len(groups), 5.0)
    else:
        nrows, ncols = 2, 2
        figsize = (13, 9)

    fig, axes = plt.subplots(nrows, ncols, figsize=figsize, sharey=True)
    axes_list = list(axes.flat) if hasattr(axes, "flat") else [axes]
    title_fs = 18 if large_text else 13
    panel_title_fs = 16 if large_text else 11
    label_fs = 14 if large_text else 9
    axis_label_fs = 15 if large_text else None
    tick_fs = 13 if large_text else None
    fig.suptitle(title, fontsize=title_fs, y=0.995)
    for ax, (grp, labs, vals) in zip(axes_list, panels):
        if not labs:
            ax.set_visible(False)
            continue
        offset = (max_slots - len(labs)) / 2
        positions = [offset + i + 1 for i in range(len(labs))]
        colors = [IMPL_COLORS.get(lab, "#777777") for lab in labs]
        _bar_with_box(ax, positions, vals, colors)
        _annotate_polars_finalize(ax, labs, positions, vals, finalize_ns, grp)
        ax.set_xticks(positions)
        ax.set_xticklabels(labs, rotation=22, ha="right", fontsize=label_fs)
        ax.set_title(GROUP_TITLES.get(grp, grp), fontsize=panel_title_fs, fontweight="bold")
        ax.set_xlim(0.5, max_slots + 0.5)
        if log:
            ax.set_yscale("log")
        ax.set_ylim(ymin, ymax)
        ax.yaxis.set_major_formatter(mticker.FuncFormatter(fmt_mops))
        ax.tick_params(axis="y", labelsize=tick_fs)
        ax.grid(axis="y", linestyle=":", alpha=0.6, which="both" if log else "major")
    for ax in axes_list[len(panels):]:
        ax.set_visible(False)
    for ax in axes_list[::ncols]:
        ax.set_ylabel(ylabel, fontsize=axis_label_fs)
    fig.tight_layout(rect=(0, 0, 1, 0.97))
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def cms_cs_2x2(data: dict, impls: list[str], *,
               title: str, ylabel: str, out_png: Path,
               log: bool = False, finalize_ns: dict | None = None) -> None:
    cms_cs_grid(data, impls,
                groups=["CMS@2K", "CMS@32K", "CS@2K", "CS@32K"],
                title=title,
                ylabel=ylabel,
                out_png=out_png,
                log=log,
                finalize_ns=finalize_ns)


# ---------- Impl lists per group ----------

HLL_SKETCH_INSERT = ["asap_sketchlib (HIP)", "oxide",
                     "datasketches (Rust)", "datasketches (C++)"]
HLL_SKETCH_QUERY = HLL_SKETCH_INSERT[:]

KLL_SKETCH_INSERT = ["asap_sketchlib", "oxide",
                     "datasketches (C++ Apache)", "InsertOptimized"]
KLL_SKETCH_QUERY = ["asap_sketchlib", "oxide", "datasketches (C++ Apache)"]

CMS_CS_SKETCH_INSERT = ["asap_sketchlib", "oxide",
                        "datasketches (Rust)", "datasketches (C++)",
                        "InsertOptimized"]
CMS_CS_SKETCH_INSERT_NO_INSERT_OPT = [
    impl for impl in CMS_CS_SKETCH_INSERT if impl != "InsertOptimized"
]
CMS_CS_SKETCH_QUERY = ["asap_sketchlib", "oxide",
                       "datasketches (Rust)", "datasketches (C++)"]

VS_POLARS_HLL = ["asap_sketchlib (HIP)", "polars (exact)"]
VS_POLARS_KLL = ["asap_sketchlib", "polars (exact)"]
VS_POLARS_CMS_CS = ["asap_sketchlib", "polars (exact)"]


def main() -> int:
    cms_ins = load_long(OUT_DIR / "cms_cs.csv", "throughput_items_per_sec")
    cms_qry = load_long(OUT_DIR / "cms_cs_query.csv", "throughput_queries_per_sec")
    hll_ins = load_long(OUT_DIR / "hll.csv", "throughput_items_per_sec")
    hll_qry = load_long(OUT_DIR / "hll_query.csv", "throughput_queries_per_sec")
    kll_ins = load_long(OUT_DIR / "kll.csv", "throughput_items_per_sec")
    kll_qry = load_long(OUT_DIR / "kll_query.csv", "throughput_queries_per_sec")

    # Finalize wall time per (group, impl). Used to annotate `+sort/+nunique/
    # +groupby` overhead on polars insert bars in vs_polars charts so the
    # reader sees the real exact-baseline cost (insert bar = Vec::push only).
    cms_fin = load_long(OUT_DIR / "cms_cs_finalize.csv", "finalize_nanoseconds")
    hll_fin = load_long(OUT_DIR / "hll_finalize.csv", "finalize_nanoseconds")
    kll_fin = load_long(OUT_DIR / "kll_finalize.csv", "finalize_nanoseconds")

    note = "Zipf s=1.1, 10M items, 10 runs"

    # ----- insertion comparisons (sketch vs sketch) -----
    cms_cs_2x2(cms_ins, CMS_CS_SKETCH_INSERT,
               title=f"CMS / CountSketch insertion throughput ({note})",
               ylabel="Throughput (items/sec)",
               out_png=INSERT_DIR / "cms_cs.png")
    cms_cs_2x2(cms_ins, CMS_CS_SKETCH_INSERT_NO_INSERT_OPT,
               title=f"CMS / CountSketch insertion throughput, no InsertOptimized ({note})",
               ylabel="Throughput (items/sec)",
               out_png=INSERT_DIR / "cms_cs_no_insertoptimized.png")
    cms_cs_grid(cms_ins, CMS_CS_SKETCH_INSERT,
                groups=["CMS@32K", "CS@32K"],
                title=f"CMS / CountSketch insertion throughput, 32K only ({note})",
                ylabel="Throughput (items/sec)",
                out_png=INSERT_DIR / "cms_cs_32k.png",
                large_text=True)
    cms_cs_grid(cms_ins, CMS_CS_SKETCH_INSERT,
                groups=["CMS@2K", "CS@2K"],
                title=f"CMS / CountSketch insertion throughput, 2K only ({note})",
                ylabel="Throughput (items/sec)",
                out_png=INSERT_DIR / "cms_cs_2k.png")
    single_panel(hll_ins, "HLL", HLL_SKETCH_INSERT,
                 title=f"HLL insertion throughput @ lg_k=14 ({note})",
                 ylabel="Throughput (items/sec)",
                 out_png=INSERT_DIR / "hll.png")
    single_panel(kll_ins, "KLL", KLL_SKETCH_INSERT,
                 title=f"KLL insertion throughput @ k=200 ({note})",
                 ylabel="Throughput (items/sec)",
                 out_png=INSERT_DIR / "kll.png")

    # ----- query comparisons (sketch vs sketch) -----
    cms_cs_2x2(cms_qry, CMS_CS_SKETCH_QUERY,
               title=f"CMS / CountSketch query throughput ({note})",
               ylabel="Throughput (queries/sec)",
               out_png=QUERY_DIR / "cms_cs.png")
    cms_cs_2x2(cms_qry, CMS_CS_SKETCH_QUERY,
               title=f"CMS / CountSketch query throughput, no InsertOptimized ({note})",
               ylabel="Throughput (queries/sec)",
               out_png=QUERY_DIR / "cms_cs_no_insertoptimized.png")
    cms_cs_grid(cms_qry, CMS_CS_SKETCH_QUERY,
                groups=["CMS@32K", "CS@32K"],
                title=f"CMS / CountSketch query throughput, 32K only ({note})",
                ylabel="Throughput (queries/sec)",
                out_png=QUERY_DIR / "cms_cs_32k.png",
                large_text=True)
    cms_cs_grid(cms_qry, CMS_CS_SKETCH_QUERY,
                groups=["CMS@2K", "CS@2K"],
                title=f"CMS / CountSketch query throughput, 2K only ({note})",
                ylabel="Throughput (queries/sec)",
                out_png=QUERY_DIR / "cms_cs_2k.png")
    single_panel(hll_qry, "HLL", HLL_SKETCH_QUERY,
                 title=f"HLL query throughput @ lg_k=14 ({note}) — log scale",
                 ylabel="Throughput (queries/sec, log)",
                 out_png=QUERY_DIR / "hll.png",
                 log=True)
    single_panel(kll_qry, "KLL", KLL_SKETCH_QUERY,
                 title=f"KLL query throughput @ k=200 ({note}) — log scale",
                 ylabel="Throughput (queries/sec, log)",
                 out_png=QUERY_DIR / "kll.png",
                 log=True)

    # ----- sketchlib vs polars (exact) -----
    # Insert charts annotate polars bars with `+sort/+nunique/+groupby: X` so
    # the reader sees the heavyweight finalize cost that the bar itself
    # excludes (polars `update()` is just `Vec::push`).
    vs_polars_subtitle = "\npolars insert bar = Vec::push only; +finalize cost annotated above bar"
    cms_cs_2x2(cms_ins, VS_POLARS_CMS_CS,
               title=f"asap_sketchlib vs polars (exact) — CMS/CS insertion ({note}){vs_polars_subtitle}",
               ylabel="Throughput (items/sec)",
               out_png=VS_POLARS_DIR / "cms_cs_insert.png",
               finalize_ns=cms_fin)
    single_panel(cms_ins, "CMS@2K", VS_POLARS_CMS_CS,
                 title=f"CountMin<FixedMatrix, FastPath> vs polars (exact) — insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "cms_fixedmatrix_fast_insert.png",
                 finalize_ns=cms_fin)
    single_panel(cms_ins, "CMS@32K", VS_POLARS_CMS_CS,
                 title=f"CountMin<FixedMatrix5x32K, FastPath> vs polars (exact) — insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "cms_fixedmatrix_fast_32k_insert.png",
                 finalize_ns=cms_fin)
    single_panel(cms_ins, "CS@32K", VS_POLARS_CMS_CS,
                 title=f"CountSketch<FixedMatrix5x32K, FastPath> vs polars (exact) — insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "cs_fixedmatrix_fast_32k_insert.png",
                 finalize_ns=cms_fin)
    cms_cs_2x2(cms_qry, VS_POLARS_CMS_CS,
               title=f"asap_sketchlib vs polars (exact) — CMS/CS query ({note})",
               ylabel="Throughput (queries/sec)",
               out_png=VS_POLARS_DIR / "cms_cs_query.png")
    single_panel(cms_qry, "CMS@2K", VS_POLARS_CMS_CS,
                 title=f"CountMin<FixedMatrix, FastPath> vs polars (exact) — query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "cms_fixedmatrix_fast_query.png")
    single_panel(cms_qry, "CMS@32K", VS_POLARS_CMS_CS,
                 title=f"CountMin<FixedMatrix5x32K, FastPath> vs polars (exact) — query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "cms_fixedmatrix_fast_32k_query.png")
    single_panel(cms_qry, "CS@32K", VS_POLARS_CMS_CS,
                 title=f"CountSketch<FixedMatrix5x32K, FastPath> vs polars (exact) — query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "cs_fixedmatrix_fast_32k_query.png")
    cms_cs_32k_accuracy(VS_POLARS_DIR / "cms_cs_32k_accuracy.png")
    latency_composition(
        cms_ins, cms_fin, cms_qry,
        group="CMS@32K",
        sketch_impl="asap_sketchlib",
        sketch_label="asap_sketchlib CMS (5x32768)",
        polars_label="Polars exact",
        finalize_label="GroupBy",
        title="CMS Frequency Workflow Latency",
        out_png=VS_POLARS_DIR / "cms_32k_latency_composition.png",
        query_count=query_count_from_csv(
            RAW_CMS_CS_DIR / "cms__lib_fixedmatrix_fast_32k__rows5_cols32768" / "cms_throughput_query_results_rust.csv",
            99_778,
        ),
    )
    latency_composition(
        cms_ins, cms_fin, cms_qry,
        group="CS@32K",
        sketch_impl="asap_sketchlib",
        sketch_label="asap_sketchlib CountSketch (5x32768)",
        polars_label="Polars exact",
        finalize_label="GroupBy",
        title="CountSketch Frequency Workflow Latency",
        out_png=VS_POLARS_DIR / "cs_32k_latency_composition.png",
        query_count=query_count_from_csv(
            RAW_CMS_CS_DIR / "countsketch__lib_fixedmatrix_fast_32k__rows5_cols32768" / "countsketch_throughput_query_results_rust.csv",
            99_778,
        ),
    )
    single_panel(hll_ins, "HLL", VS_POLARS_HLL,
                 title=f"asap_sketchlib (HIP) vs polars (exact) — HLL insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "hll_insert.png",
                 finalize_ns=hll_fin)
    single_panel(hll_qry, "HLL", VS_POLARS_HLL,
                 title=f"asap_sketchlib (HIP) vs polars (exact) — HLL query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "hll_query.png")
    latency_composition(
        hll_ins, hll_fin, hll_qry,
        group="HLL",
        sketch_impl="asap_sketchlib (HIP)",
        sketch_label="asap_sketchlib HLL (HIP)",
        polars_label="Polars exact",
        finalize_label="n_unique",
        title="HLL Cardinality Workflow Latency",
        out_png=VS_POLARS_DIR / "hll_latency_composition.png",
        query_count=query_count_from_csv(
            OUT_DIR / "raw" / "hll" / "hll__lib_hip__lg_k14" / "hll_throughput_query_tight_results_rust.csv",
            4_096,
        ),
    )
    single_panel(kll_ins, "KLL", VS_POLARS_KLL,
                 title=f"asap_sketchlib vs polars (exact) — KLL insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "kll_insert.png",
                 finalize_ns=kll_fin)
    single_panel(kll_qry, "KLL", VS_POLARS_KLL,
                 title=f"asap_sketchlib vs polars (exact) — KLL query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "kll_query.png")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
