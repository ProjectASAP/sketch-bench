#!/usr/bin/env python3
"""Reorganise the throughput_boxplot_v2 figures into three thematic groups,
reading the existing per-panel long-form CSVs (no benchmarks re-run here).

Groups:
  * `insert/`     — cross-impl insertion throughput, one figure per family.
                    polars is excluded; it belongs in `vs_polars/`.
  * `query/`      — cross-impl query throughput, one figure per family.
                    HLL Classic is dropped (Classic vs HIP story lives in the
                    raw CSV). KLL uses a log y-axis because oxide's quantile()
                    is O(num_levels) per call and runs ~4 decades slower than
                    the cached / amortised alternatives.
  * `vs_polars/`  — asap_sketchlib vs polars(exact) only. One figure per
                    family × {insert, query}. CMS/CS uses a 2×2 sub-grid
                    over the four shape configs.

CMS/CS panels are rendered as 2×2 sub-grids (CMS@2K, CMS@32K, CS@2K, CS@32K)
so the four shape configs are easy to compare without making one figure
ridiculously wide.
"""
from __future__ import annotations

import csv
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.ticker as mticker

REPO = Path(__file__).resolve().parents[1]
OUT_DIR = REPO / "visualization" / "output" / "throughput_boxplot_v2"

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


def load_long(csv_path: Path, value_col: str) -> dict[tuple[str, str], list[float]]:
    out: dict[tuple[str, str], list[float]] = defaultdict(list)
    if not csv_path.exists():
        return {}
    with csv_path.open() as f:
        for r in csv.DictReader(f):
            out[(r["group"], r["impl"])].append(float(r[value_col]))
    return dict(out)


def fmt_mops(v: float, _pos=None) -> str:
    if v >= 1e6:
        return f"{v / 1e6:.0f}M"
    if v >= 1e3:
        return f"{v / 1e3:.0f}k"
    return f"{v:.0f}"


def _bar_with_box(ax, positions, values, colors, *, bar_w=0.62, box_w=0.22) -> None:
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
    op_by_family = {
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
        op = op_by_family.get(group, "+finalize")
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


def cms_cs_2x2(data: dict, impls: list[str], *,
               title: str, ylabel: str, out_png: Path,
               log: bool = False, finalize_ns: dict | None = None) -> None:
    groups = ["CMS@2K", "CMS@32K", "CS@2K", "CS@32K"]
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

    fig, axes = plt.subplots(2, 2, figsize=(13, 9), sharey=True)
    fig.suptitle(title, fontsize=13, y=0.995)
    for ax, (grp, labs, vals) in zip(axes.flat, panels):
        if not labs:
            ax.set_visible(False)
            continue
        positions = list(range(1, len(labs) + 1))
        colors = [IMPL_COLORS.get(lab, "#777777") for lab in labs]
        _bar_with_box(ax, positions, vals, colors)
        _annotate_polars_finalize(ax, labs, positions, vals, finalize_ns, grp)
        ax.set_xticks(positions)
        ax.set_xticklabels(labs, rotation=22, ha="right", fontsize=9)
        ax.set_title(grp, fontsize=11, fontweight="bold")
        if log:
            ax.set_yscale("log")
        ax.set_ylim(ymin, ymax)
        ax.yaxis.set_major_formatter(mticker.FuncFormatter(fmt_mops))
        ax.grid(axis="y", linestyle=":", alpha=0.6, which="both" if log else "major")
    for ax in axes[:, 0]:
        ax.set_ylabel(ylabel)
    fig.tight_layout(rect=(0, 0, 1, 0.97))
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


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
    cms_cs_2x2(cms_qry, VS_POLARS_CMS_CS,
               title=f"asap_sketchlib vs polars (exact) — CMS/CS query ({note})",
               ylabel="Throughput (queries/sec)",
               out_png=VS_POLARS_DIR / "cms_cs_query.png")
    single_panel(hll_ins, "HLL", VS_POLARS_HLL,
                 title=f"asap_sketchlib (HIP) vs polars (exact) — HLL insertion ({note}){vs_polars_subtitle}",
                 ylabel="Throughput (items/sec)",
                 out_png=VS_POLARS_DIR / "hll_insert.png",
                 finalize_ns=hll_fin)
    single_panel(hll_qry, "HLL", VS_POLARS_HLL,
                 title=f"asap_sketchlib (HIP) vs polars (exact) — HLL query ({note})",
                 ylabel="Throughput (queries/sec)",
                 out_png=VS_POLARS_DIR / "hll_query.png")
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
