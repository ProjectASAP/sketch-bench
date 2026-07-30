#!/usr/bin/env python3
"""Throughput box-plot driver — v2.

Addresses issue #23 (cross-impl cache/allocator contamination dragging
sketchlib numbers low) by:

  * Phase 1: run every sketchlib panel/shape in isolation FIRST, while the
    machine is quiet.
  * Phase 2: run baselines (oxide / datasketches Rust / datasketches C++ /
    InsertOpt-Final / polars exact) afterward.
  * Each invocation is its own fresh process, pinned via `taskset -c $PIN_CORE`
    with `BENCH_WARMUP_SECS=30` (matches PR #21's validated config).
  * --warmup-runs bumped from 3 → 5 for CMS/CS/HLL, 7 for KLL.
  * cpp-bench binaries already split throughput/latency passes per PR #22 and
    burn-warmup the governor.

Output figures (per the user's spec):
  cms_cs.png        — 4 groups: CMS@2K | CMS@32K | CS@2K | CS@32K
  hll.png           — HLL lg_k=14
  kll.png           — KLL k=200
  cms_cs_query.png  — query-throughput counterpart (InsertOptimized dropped:
                      no query API)
  hll_query.png
  kll_query.png

The polars wrapper buffers items in `update()` and does the actual
DataFrame work in `finalize_for_query`. The runner only times the insert
loop, so the polars insert bar reflects ~Vec<i64>::push throughput, not
the end-to-end exact-baseline cost. Query bars reflect the post-finalize
HashMap / array lookup. Read accordingly.

All raw per-run samples are written to CSV alongside the PNGs so plots can be
reproduced without re-running benches.
"""
from __future__ import annotations

import argparse
import csv
import os
import subprocess
import sys
from pathlib import Path

import matplotlib.pyplot as plt

REPO = Path(__file__).resolve().parents[1]
INPUT_BIN = REPO / "input" / "benchmark_data_10m_int64_zipf_s11_k100000.bin"
OUT_DIR = REPO / "visualization" / "output" / "throughput_boxplot_v2"
RAW_DIR = OUT_DIR / "raw"
CPP_BUILD = REPO / "cpp-bench" / "build"

RUNS = 10
WARMUP_RUNS_DEFAULT = 5
WARMUP_RUNS_KLL = 7
MEASURE_ITEMS = 10_000_000
WARMUP_ITEMS = 100_000
PIN_CORE = os.environ.get("PIN_CORE", "2")
BENCH_WARMUP_SECS = os.environ.get("BENCH_WARMUP_SECS", "30")

ENV = {**os.environ, "BENCH_WARMUP_SECS": BENCH_WARMUP_SECS}

OUT_DIR.mkdir(parents=True, exist_ok=True)
RAW_DIR.mkdir(parents=True, exist_ok=True)


def run(cmd: list[str]) -> None:
    print("$", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run(cmd, check=True, env=ENV)


def taskset(cmd: list[str]) -> list[str]:
    return ["taskset", "-c", PIN_CORE, *cmd]


def cli_bench(algorithm: str, impl: str, config: str, panel_dir: Path, *,
              warmup_runs: int = WARMUP_RUNS_DEFAULT,
              with_query: bool = True) -> tuple[Path, Path | None]:
    """Run a aqpbm-cli bench. Returns (insert_csv, query_csv_or_None)."""
    panel_dir.mkdir(parents=True, exist_ok=True)
    impl_dir = panel_dir / f"{algorithm}__{impl.replace('-', '_')}__{config.replace(' ', '_').replace('=', '')}"
    if impl_dir.exists():
        for f in impl_dir.iterdir():
            f.unlink()
    impl_dir.mkdir(parents=True, exist_ok=True)
    cmd = taskset([
        "cargo", "run", "--release", "--quiet", "-p", "aqpbm-cli", "--",
        "bench",
        "--sketch", algorithm,
        "--impl", impl,
        "--input", str(INPUT_BIN),
        "--runs", str(RUNS),
        "--warmup-runs", str(warmup_runs),
        "--config", config,
        "--raw-csv", str(impl_dir),
        "--report", str(impl_dir / "report.jsonl"),
    ])
    if with_query:
        cmd += ["--accuracy"]
    run(cmd)
    insert_csv = impl_dir / f"{algorithm}_throughput_results_rust.csv"
    # Per-call CSV (one row per timed call). Used for accuracy / latency
    # distributions; the per-call clock_gettime overhead is baked in, so
    # it is NOT apples-to-apples with cpp-bench's tight-loop query CSV.
    query_csv = impl_dir / f"{algorithm}_throughput_query_results_rust.csv"
    # Tight-loop CSV (one row per run, comparator's outer Instant pair).
    # Apples-to-apples with cpp-bench `--query-csv`.
    query_tight = impl_dir / f"{algorithm}_throughput_query_tight_results_rust.csv"
    chosen = query_tight if (with_query and query_tight.exists()) else query_csv
    return insert_csv, (chosen if with_query and chosen.exists() else None)


def cpp_bench(binary: Path, k: int | None, panel_dir: Path, tag: str, *,
              with_query: bool = False, with_query_percall: bool = False) -> tuple[Path, Path | None]:
    panel_dir.mkdir(parents=True, exist_ok=True)
    csv_path = panel_dir / f"cpp_{tag}.csv"
    qcsv_path = panel_dir / f"cpp_{tag}_query.csv"
    pcsv_path = panel_dir / f"cpp_{tag}_query_percall.csv"
    cmd = taskset([
        str(binary),
        "--workload-file", str(INPUT_BIN),
        "--runs", str(RUNS),
        "--warmup-items", str(WARMUP_ITEMS),
        "--measure-items", str(MEASURE_ITEMS),
        "--latency-stride", "0",
        "--report", "/dev/null",
        "--legacy-csv", str(csv_path),
    ])
    if k is not None:
        cmd += ["--k", str(k)]
    if with_query:
        cmd += ["--query-csv", str(qcsv_path)]
    if with_query_percall:
        cmd += ["--query-percall-csv", str(pcsv_path)]
    run(cmd)
    return csv_path, (qcsv_path if with_query and qcsv_path.exists() else None)


def read_throughputs(csv_path: Path) -> list[float]:
    out = []
    with csv_path.open() as f:
        reader = csv.DictReader(f)
        for row in reader:
            out.append(float(row["throughput_items_per_sec"]))
    return out


def read_finalize_ns(csv_path: Path) -> list[float]:
    """Read per-run finalize_for_query wall time (ns) from the insert CSV.

    Returns [] for any CSV missing the column — older / cpp-bench rows do
    not have it. Callers should treat empty as "unknown / not measured".
    """
    out = []
    with csv_path.open() as f:
        reader = csv.DictReader(f)
        for row in reader:
            v = row.get("finalize_nanoseconds")
            if v is None or v == "":
                return []
            try:
                out.append(float(v))
            except ValueError:
                return []
    return out


def read_query_throughputs(csv_path: Path) -> list[float]:
    """Read per-run query throughput.

    Two schemas are accepted:
      * Aggregate (CMS / CountSketch / cpp-bench): one row per run with
        `throughput_queries_per_sec` directly.
      * Per-call (HLL / KLL / DD legacy schema): N sampled calls per run with
        a `nanoseconds` column. We aggregate by the `run` column —
        throughput = calls / sum(nanoseconds).
    """
    with csv_path.open() as f:
        reader = csv.DictReader(f)
        rows = list(reader)
    if not rows:
        return []
    if "throughput_queries_per_sec" in rows[0]:
        return [float(r["throughput_queries_per_sec"]) for r in rows]
    if "nanoseconds" in rows[0]:
        by_run: dict[str, list[float]] = {}
        for r in rows:
            by_run.setdefault(r["run"], []).append(float(r["nanoseconds"]))
        # Preserve insertion order (Python 3.7+ dicts are ordered).
        out = []
        for ns_list in by_run.values():
            total_ns = sum(ns_list)
            if total_ns > 0:
                out.append(len(ns_list) * 1e9 / total_ns)
        return out
    raise ValueError(f"unknown query CSV schema in {csv_path}: columns={list(rows[0])}")


# ---------- Phased execution ----------
#
# Each step is (panel_tag, group, impl_label, runner, [aliases]).
# `runner` returns (insert_csv, query_csv_or_None).
# `aliases` is an optional list of extra (panel, group, label) tuples that
# should receive a copy of this step's results — used for polars, which is
# config-independent (one run per algorithm, replicated across width subgroups).

PANEL_CMS_CS = "cms_cs"
PANEL_HLL = "hll"
PANEL_KLL = "kll"

SKETCHLIB_STEPS = [
    (PANEL_CMS_CS, "CMS@2K",  "asap_sketchlib",
     lambda: cli_bench("cms-fastpath-fixedmatrix",  "lib", "rows=5 cols=2048",  RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CMS@32K", "asap_sketchlib",
     lambda: cli_bench("cms-fastpath-fixedmatrix", "lib", "rows=5 cols=32768", RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CS@2K",   "asap_sketchlib",
     lambda: cli_bench("countsketch-fastpath-fixedmatrix",  "lib", "rows=5 cols=2048",  RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CS@32K",  "asap_sketchlib",
     lambda: cli_bench("countsketch-fastpath-fixedmatrix", "lib", "rows=5 cols=32768", RAW_DIR / "cms_cs"), []),
    (PANEL_HLL,    "HLL",     "asap_sketchlib (Classic)",
     lambda: cli_bench("hll",         "lib",                      "lg_k=14",           RAW_DIR / "hll"), []),
    (PANEL_HLL,    "HLL",     "asap_sketchlib (HIP)",
     lambda: cli_bench("hll-hip",     "lib", "lg_k=14",           RAW_DIR / "hll"), []),
    (PANEL_KLL,    "KLL",     "asap_sketchlib",
     lambda: cli_bench("kll-cdf",     "lib", "k=200",             RAW_DIR / "kll",
                       warmup_runs=WARMUP_RUNS_KLL), []),
]

BASELINE_STEPS = [
    # CMS/CS panel
    (PANEL_CMS_CS, "CMS@2K",  "oxide",
     lambda: cli_bench("cms",         "oxide",        "rows=5 cols=2048",  RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CMS@2K",  "datasketches (Rust)",
     lambda: cli_bench("cms",         "datasketches", "rows=5 cols=2048",  RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CMS@2K",  "datasketches (C++)",
     lambda: cpp_bench(CPP_BUILD / "cms" / "datasketches_cms", 2048,  RAW_DIR / "cms_cs", "ds_cms_2k",
                       with_query=True), []),
    (PANEL_CMS_CS, "CMS@32K", "oxide",
     lambda: cli_bench("cms",         "oxide",        "rows=5 cols=32768", RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CMS@32K", "datasketches (Rust)",
     lambda: cli_bench("cms",         "datasketches", "rows=5 cols=32768", RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CMS@32K", "datasketches (C++)",
     lambda: cpp_bench(CPP_BUILD / "cms" / "datasketches_cms", 32768, RAW_DIR / "cms_cs", "ds_cms_32k",
                       with_query=True), []),
    (PANEL_CMS_CS, "CS@2K",   "oxide",
     lambda: cli_bench("countsketch", "oxide",        "rows=5 cols=2048",  RAW_DIR / "cms_cs"), []),
    (PANEL_CMS_CS, "CS@2K",   "InsertOptimized",
     lambda: cpp_bench(CPP_BUILD / "cs"  / "final_cs", None, RAW_DIR / "cms_cs", "final_cs_2k"), []),
    (PANEL_CMS_CS, "CS@32K",  "oxide",
     lambda: cli_bench("countsketch", "oxide",        "rows=5 cols=32768", RAW_DIR / "cms_cs"), []),
    # Polars exact baseline (config-independent — one run replicated across
    # the width subgroups in each algorithm).
    (PANEL_CMS_CS, "CMS@2K",  "polars (exact)",
     lambda: cli_bench("cms",         "polars",       "rows=5 cols=2048",  RAW_DIR / "cms_cs"),
     [(PANEL_CMS_CS, "CMS@32K", "polars (exact)")]),
    (PANEL_CMS_CS, "CS@2K",   "polars (exact)",
     lambda: cli_bench("countsketch", "polars",       "rows=5 cols=2048",  RAW_DIR / "cms_cs"),
     [(PANEL_CMS_CS, "CS@32K",  "polars (exact)")]),
    # HLL
    (PANEL_HLL,    "HLL",     "oxide",
     lambda: cli_bench("hll",         "oxide",        "lg_k=14",           RAW_DIR / "hll"), []),
    (PANEL_HLL,    "HLL",     "datasketches (Rust)",
     lambda: cli_bench("hll",         "datasketches", "lg_k=14",           RAW_DIR / "hll"), []),
    (PANEL_HLL,    "HLL",     "datasketches (C++)",
     lambda: cpp_bench(CPP_BUILD / "hll" / "datasketches_hll", 14,    RAW_DIR / "hll", "ds_hll",
                       with_query=True, with_query_percall=True), []),
    (PANEL_HLL,    "HLL",     "polars (exact)",
     lambda: cli_bench("hll",         "polars",       "lg_k=14",           RAW_DIR / "hll"), []),
    # KLL — no C++ DataSketches query (Insert-Opt fork has no quantile API);
    # final_kll is InsertOptimized (also no query).
    (PANEL_KLL,    "KLL",     "oxide",
     lambda: cli_bench("kll-cdf",     "oxide", "k=200",             RAW_DIR / "kll",
                       warmup_runs=WARMUP_RUNS_KLL), []),
    (PANEL_KLL,    "KLL",     "datasketches (C++)",
     lambda: cpp_bench(CPP_BUILD / "kll" / "datasketches_kll", 200,   RAW_DIR / "kll", "ds_kll"), []),
    (PANEL_KLL,    "KLL",     "datasketches (C++ Apache)",
     lambda: cpp_bench(CPP_BUILD / "kll" / "datasketches_upstream_kll", 200,
                       RAW_DIR / "kll", "ds_upstream_kll",
                       with_query=True, with_query_percall=True), []),
    (PANEL_KLL,    "KLL",     "InsertOptimized",
     lambda: cpp_bench(CPP_BUILD / "kll" / "final_kll",        200,   RAW_DIR / "kll", "final_kll"), []),
    (PANEL_KLL,    "KLL",     "polars (exact)",
     lambda: cli_bench("kll",         "polars",       "k=200",             RAW_DIR / "kll",
                       warmup_runs=WARMUP_RUNS_KLL), []),
]


# ---------- Plotting ----------

def fmt_mops(v: float, _pos=None) -> str:
    return f"{v / 1e6:.0f}M"


def _bar_with_mini_box(ax, positions, values, colors) -> None:
    means = [sum(v) / len(v) for v in values]
    ax.bar(positions, means, width=0.62, color=colors, alpha=0.55,
           edgecolor="black", linewidth=0.6, zorder=1)
    bp = ax.boxplot(values, positions=positions, widths=0.22, showfliers=False,
                    patch_artist=True, zorder=3,
                    medianprops=dict(color="black", linewidth=1.2),
                    whiskerprops=dict(color="black", linewidth=0.8),
                    capprops=dict(color="black", linewidth=0.8),
                    boxprops=dict(linewidth=0.8))
    for patch in bp["boxes"]:
        patch.set_facecolor("white")
        patch.set_alpha(0.95)


IMPL_COLORS = {
    "asap_sketchlib":            "#1f77b4",
    "asap_sketchlib (Classic)":  "#1f77b4",
    "asap_sketchlib (HIP)":      "#17becf",
    "oxide":                     "#ff7f0e",
    "datasketches (Rust)":       "#2ca02c",
    "datasketches (C++)":        "#d62728",
    "InsertOptimized":           "#9467bd",
    "polars (exact)":            "#8c564b",
}


def _annotate_finalize(ax, keys, positions, values, finalize_ns) -> None:
    """Stamp '+sort: X ms/run' above bars whose finalize_ns is non-trivial.

    finalize_ns is {(group, impl): [ns per run]}. We only annotate when the
    mean finalize cost is > 1% of the mean bar value's per-run wall time
    (i.e. visibly contaminates the displayed bar). Bars without finalize
    data (cpp-bench) are silently skipped.
    """
    if not finalize_ns:
        return
    for (k, p, v) in zip(keys, positions, values):
        ns_list = finalize_ns.get(k)
        if not ns_list:
            continue
        mean_ns = sum(ns_list) / len(ns_list)
        if mean_ns < 1e6:  # < 1 ms — uninteresting (HLL/exact baselines)
            continue
        bar_top = sum(v) / len(v)
        mean_ms = mean_ns / 1e6
        label = f"+sort\n{mean_ms / 1000:.2f} s" if mean_ms >= 1000 else f"+sort\n{mean_ms:.1f} ms"
        ax.annotate(label,
                    xy=(p, bar_top), xytext=(0, 6), textcoords="offset points",
                    ha="center", va="bottom", fontsize=7.5,
                    color="#444444",
                    bbox=dict(boxstyle="round,pad=0.18", facecolor="#fff5cc",
                              edgecolor="#c9b400", linewidth=0.5, alpha=0.9))


def boxplot_grouped(data: dict, title: str, out_png: Path,
                    group_order: list[str], ylabel: str,
                    finalize_ns: dict | None = None) -> None:
    """data is {(group, impl): [throughputs]}."""
    keys = []
    for grp in group_order:
        for (g, lab), _ in data.items():
            if g == grp and (g, lab) not in keys:
                keys.append((g, lab))
    positions = []
    pos = 1.0
    last_group = None
    for grp, _ in keys:
        if last_group is not None and grp != last_group:
            pos += 1.0
        positions.append(pos)
        pos += 1.0
        last_group = grp
    values = [data[k] for k in keys]
    labels = [lab for _, lab in keys]
    colors = [IMPL_COLORS.get(lab, "#777777") for lab in labels]

    fig, ax = plt.subplots(figsize=(max(8, 1.05 * len(keys) + 2), 5.2))
    _bar_with_mini_box(ax, positions, values, colors)
    _annotate_finalize(ax, keys, positions, values, finalize_ns)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=22, ha="right", fontsize=9)
    group_centers: dict[str, list[float]] = {}
    for (grp, _), p in zip(keys, positions):
        group_centers.setdefault(grp, []).append(p)
    for grp, ps in group_centers.items():
        cx = sum(ps) / len(ps)
        ax.annotate(grp, xy=(cx, 0), xytext=(0, -78), textcoords="offset points",
                    ha="center", fontsize=11, fontweight="bold",
                    xycoords=("data", "axes fraction"))
    ax.set_title(title)
    ax.set_ylabel(ylabel)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_mops))
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    fig.tight_layout()
    fig.subplots_adjust(bottom=0.30)
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def boxplot_single(data: dict, title: str, out_png: Path,
                   impl_order: list[str], ylabel: str,
                   finalize_ns: dict | None = None) -> None:
    """data is {(group, impl): [throughputs]} with a single group."""
    grp = next(iter(data.keys()))[0]
    keys = [(grp, lab) for lab in impl_order if (grp, lab) in data]
    values = [data[k] for k in keys]
    labels = [lab for _, lab in keys]
    colors = [IMPL_COLORS.get(lab, "#777777") for lab in labels]
    positions = list(range(1, len(keys) + 1))
    fig, ax = plt.subplots(figsize=(max(6, 1.4 * len(keys) + 2), 4.8))
    _bar_with_mini_box(ax, positions, values, colors)
    _annotate_finalize(ax, keys, positions, values, finalize_ns)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=18, ha="right")
    ax.set_title(title)
    ax.set_ylabel(ylabel)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_mops))
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    fig.tight_layout()
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def boxplot_finalize(data: dict, title: str, out_png: Path,
                     impl_order: list[str]) -> None:
    """Finalize-wall (ms/run) box plot. data is {(group, impl): [ns]}."""
    if not data:
        return
    grp = next(iter(data.keys()))[0]
    keys = [(grp, lab) for lab in impl_order if (grp, lab) in data and data[(grp, lab)]]
    if not keys:
        return
    # Convert ns -> ms for display.
    values = [[v / 1e6 for v in data[k]] for k in keys]
    labels = [lab for _, lab in keys]
    colors = [IMPL_COLORS.get(lab, "#777777") for lab in labels]
    positions = list(range(1, len(keys) + 1))
    fig, ax = plt.subplots(figsize=(max(6, 1.4 * len(keys) + 2), 4.8))
    means = [sum(v) / len(v) for v in values]
    ax.bar(positions, means, width=0.62, color=colors, alpha=0.55,
           edgecolor="black", linewidth=0.6, zorder=1)
    bp = ax.boxplot(values, positions=positions, widths=0.22, showfliers=False,
                    patch_artist=True, zorder=3,
                    medianprops=dict(color="black", linewidth=1.2),
                    whiskerprops=dict(color="black", linewidth=0.8),
                    capprops=dict(color="black", linewidth=0.8),
                    boxprops=dict(linewidth=0.8))
    for patch in bp["boxes"]:
        patch.set_facecolor("white")
        patch.set_alpha(0.95)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=18, ha="right")
    ax.set_title(title)
    ax.set_ylabel("finalize_for_query (ms/run)")
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    fig.tight_layout()
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def dump_long_csv(panel: str, data: dict, suffix: str, value_col: str) -> None:
    p = OUT_DIR / f"{panel}{suffix}.csv"
    with p.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["group", "impl", "run", value_col])
        for (grp, lab), vals in data.items():
            for i, v in enumerate(vals, 1):
                w.writerow([grp, lab, i, v])
    print(f"  wrote {p}")


HLL_INSERT_ORDER = [
    "asap_sketchlib", "asap_sketchlib (Classic)", "asap_sketchlib (HIP)",
    "oxide", "datasketches (Rust)", "datasketches (C++)", "polars (exact)",
]
HLL_QUERY_ORDER = HLL_INSERT_ORDER
KLL_INSERT_ORDER = ["asap_sketchlib", "oxide", "datasketches (C++)",
                    "datasketches (C++ Apache)", "InsertOptimized", "polars (exact)"]
KLL_QUERY_ORDER = ["asap_sketchlib", "oxide", "datasketches (C++ Apache)", "polars (exact)"]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="",
                    help="Comma-separated panel names to (re)run: cms_cs, hll, kll. "
                         "Panels not listed keep their existing top-level CSV/PNG.")
    args = ap.parse_args()
    only = {p.strip() for p in args.only.split(",") if p.strip()} if args.only else None
    if only is not None:
        allowed = {PANEL_CMS_CS, PANEL_HLL, PANEL_KLL}
        bad = only - allowed
        if bad:
            raise SystemExit(f"unknown panel(s): {bad}; valid: {sorted(allowed)}")

    panels_insert: dict[str, dict[tuple[str, str], list[float]]] = {
        PANEL_CMS_CS: {}, PANEL_HLL: {}, PANEL_KLL: {},
    }
    panels_query: dict[str, dict[tuple[str, str], list[float]]] = {
        PANEL_CMS_CS: {}, PANEL_HLL: {}, PANEL_KLL: {},
    }
    # Per-run finalize_for_query wall time (ns). Only populated for impls
    # whose insert CSV carries the `finalize_nanoseconds` column — i.e.
    # Rust impls built after the runner started timing finalize separately.
    # cpp-bench rows leave this empty.
    panels_finalize_ns: dict[str, dict[tuple[str, str], list[float]]] = {
        PANEL_CMS_CS: {}, PANEL_HLL: {}, PANEL_KLL: {},
    }

    def execute(steps, phase_name):
        print(f"\n========== Phase: {phase_name} ==========\n", flush=True)
        for entry in steps:
            panel, group, label, runner, aliases = entry
            if only is not None and panel not in only:
                continue
            print(f"--- {phase_name} :: {panel} :: {group} :: {label} ---", flush=True)
            insert_csv, query_csv = runner()
            insert_vals = read_throughputs(insert_csv)
            finalize_vals = read_finalize_ns(insert_csv)
            panels_insert[panel][(group, label)] = insert_vals
            if finalize_vals:
                panels_finalize_ns[panel][(group, label)] = finalize_vals
            for ap_p, ag, al in aliases:
                panels_insert[ap_p][(ag, al)] = list(insert_vals)
                if finalize_vals:
                    panels_finalize_ns[ap_p][(ag, al)] = list(finalize_vals)
            if query_csv is not None:
                query_vals = read_query_throughputs(query_csv)
                panels_query[panel][(group, label)] = query_vals
                for ap_p, ag, al in aliases:
                    panels_query[ap_p][(ag, al)] = list(query_vals)

    execute(SKETCHLIB_STEPS, "sketchlib (isolated)")
    execute(BASELINE_STEPS,  "baselines")

    def maybe(panel: str) -> bool:
        return only is None or panel in only

    if maybe(PANEL_CMS_CS):
        dump_long_csv(PANEL_CMS_CS, panels_insert[PANEL_CMS_CS], "", "throughput_items_per_sec")
        dump_long_csv(PANEL_CMS_CS, panels_query[PANEL_CMS_CS], "_query", "throughput_queries_per_sec")
        dump_long_csv(PANEL_CMS_CS, panels_finalize_ns[PANEL_CMS_CS], "_finalize", "finalize_nanoseconds")
        boxplot_grouped(
            panels_insert[PANEL_CMS_CS],
            "CMS / CountSketch throughput (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "cms_cs.png",
            group_order=["CMS@2K", "CMS@32K", "CS@2K", "CS@32K"],
            ylabel="Throughput (items/sec)",
        )
        boxplot_grouped(
            panels_query[PANEL_CMS_CS],
            "CMS / CountSketch query throughput (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "cms_cs_query.png",
            group_order=["CMS@2K", "CMS@32K", "CS@2K", "CS@32K"],
            ylabel="Throughput (queries/sec)",
            finalize_ns=panels_finalize_ns[PANEL_CMS_CS],
        )

    if maybe(PANEL_HLL):
        dump_long_csv(PANEL_HLL, panels_insert[PANEL_HLL], "", "throughput_items_per_sec")
        dump_long_csv(PANEL_HLL, panels_query[PANEL_HLL], "_query", "throughput_queries_per_sec")
        dump_long_csv(PANEL_HLL, panels_finalize_ns[PANEL_HLL], "_finalize", "finalize_nanoseconds")
        boxplot_single(
            panels_insert[PANEL_HLL],
            "HLL throughput @ lg_k=14 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "hll.png",
            impl_order=HLL_INSERT_ORDER,
            ylabel="Throughput (items/sec)",
        )
        boxplot_single(
            panels_query[PANEL_HLL],
            "HLL query throughput @ lg_k=14 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "hll_query.png",
            impl_order=HLL_QUERY_ORDER,
            ylabel="Throughput (queries/sec)",
            finalize_ns=panels_finalize_ns[PANEL_HLL],
        )
        boxplot_finalize(
            panels_finalize_ns[PANEL_HLL],
            "HLL finalize_for_query @ lg_k=14 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "hll_finalize.png",
            impl_order=HLL_INSERT_ORDER,
        )

    if maybe(PANEL_KLL):
        dump_long_csv(PANEL_KLL, panels_insert[PANEL_KLL], "", "throughput_items_per_sec")
        dump_long_csv(PANEL_KLL, panels_query[PANEL_KLL], "_query", "throughput_queries_per_sec")
        dump_long_csv(PANEL_KLL, panels_finalize_ns[PANEL_KLL], "_finalize", "finalize_nanoseconds")
        boxplot_single(
            panels_insert[PANEL_KLL],
            "KLL throughput @ k=200 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "kll.png",
            impl_order=KLL_INSERT_ORDER,
            ylabel="Throughput (items/sec)",
        )
        boxplot_single(
            panels_query[PANEL_KLL],
            "KLL query throughput @ k=200 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "kll_query.png",
            impl_order=KLL_QUERY_ORDER,
            ylabel="Throughput (queries/sec)",
            finalize_ns=panels_finalize_ns[PANEL_KLL],
        )
        boxplot_finalize(
            panels_finalize_ns[PANEL_KLL],
            "KLL finalize_for_query @ k=200 (Zipf s=1.1, 10M items, 10 runs)",
            OUT_DIR / "kll_finalize.png",
            impl_order=KLL_INSERT_ORDER,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
