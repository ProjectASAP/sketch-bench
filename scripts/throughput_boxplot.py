#!/usr/bin/env python3
"""Run throughput benchmarks for a fixed set of impls and emit boxplot PNGs.

Panels (per the user's request; 10M zipf s=1.1 k=100K input):
  cms_2k       : sketchlib(fm), oxide, datasketches(Rust), datasketches(C++)         @ 5x2048
  cs_2k        : sketchlib(fm), oxide, InsertOpt-Final                                @ 5x2048
  cms_cs_32k   : CMS group + CS group on one figure                                   @ 5x32768
  hll          : sketchlib(classic lib), oxide, datasketches(Rust), datasketches(C++) @ lg_k=14
  kll          : sketchlib, oxide, datasketches(C++), InsertOpt-Final                  @ k=200

All aqpbm-cli impls use --runs 10 --warmup-runs 3.
All cpp-bench binaries use --runs 10 --measure-items 10000000.
"""
from __future__ import annotations

import csv
import json
import os
import subprocess
import sys
from pathlib import Path

import matplotlib.pyplot as plt

REPO = Path(__file__).resolve().parents[1]
INPUT_BIN = REPO / "input" / "benchmark_data_10m_int64_zipf_s11_k100000.bin"
OUT_DIR = REPO / "visualization" / "output" / "throughput_boxplot"
RAW_DIR = OUT_DIR / "raw"
CPP_BUILD = REPO / "cpp-bench" / "build"

RUNS = 10
WARMUP_RUNS = 3
MEASURE_ITEMS = 10_000_000  # use the entire 10M file for cpp runner
WARMUP_ITEMS = 100_000

OUT_DIR.mkdir(parents=True, exist_ok=True)
RAW_DIR.mkdir(parents=True, exist_ok=True)


def run(cmd: list[str]) -> None:
    print("$", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run(cmd, check=True)


def cli_bench(algorithm: str, impl: str, config: str, panel_dir: Path) -> Path:
    """Run aqpbm-cli bench for one impl and return the per-run CSV path."""
    panel_dir.mkdir(parents=True, exist_ok=True)
    # raw-csv writes a fixed filename '<algorithm>_throughput_results_rust.csv'
    # so we isolate per-impl runs into per-impl subdirs to avoid clobber.
    impl_dir = panel_dir / f"{algorithm}__{impl.replace('-', '_')}__{config.replace(' ', '_').replace('=', '')}"
    if impl_dir.exists():
        for f in impl_dir.iterdir():
            f.unlink()
    impl_dir.mkdir(parents=True, exist_ok=True)
    cmd = [
        "cargo", "run", "--release", "--quiet", "-p", "aqpbm-cli", "--",
        "bench",
        "--sketch", algorithm,
        "--impl", impl,
        "--input", str(INPUT_BIN),
        "--runs", str(RUNS),
        "--warmup-runs", str(WARMUP_RUNS),
        "--config", config,
        "--raw-csv", str(impl_dir),
        "--report", str(impl_dir / "report.jsonl"),
    ]
    run(cmd)
    return impl_dir / f"{algorithm}_throughput_results_rust.csv"


def cpp_bench(binary: Path, k: int | None, panel_dir: Path, tag: str) -> Path:
    panel_dir.mkdir(parents=True, exist_ok=True)
    csv_path = panel_dir / f"cpp_{tag}.csv"
    cmd = [
        str(binary),
        "--workload-file", str(INPUT_BIN),
        "--runs", str(RUNS),
        "--warmup-items", str(WARMUP_ITEMS),
        "--measure-items", str(MEASURE_ITEMS),
        "--latency-stride", "0",
        "--report", "/dev/null",
        "--legacy-csv", str(csv_path),
    ]
    if k is not None:
        cmd += ["--k", str(k)]
    run(cmd)
    return csv_path


def read_throughputs(csv_path: Path) -> list[float]:
    out = []
    with csv_path.open() as f:
        reader = csv.DictReader(f)
        for row in reader:
            out.append(float(row["throughput_items_per_sec"]))
    return out


# ---------- Panels ----------

def panel_cms_2k() -> dict[str, list[float]]:
    pdir = RAW_DIR / "cms_2k"
    cfg = "rows=5 cols=2048"
    return {
        "sketchlib (fixed matrix)": read_throughputs(cli_bench("cms", "lib-fixedmatrix-fast", cfg, pdir)),
        "oxide":                    read_throughputs(cli_bench("cms", "oxide", cfg, pdir)),
        "datasketches (Rust)":      read_throughputs(cli_bench("cms", "datasketches", cfg, pdir)),
        "datasketches (C++)":       read_throughputs(cpp_bench(CPP_BUILD / "cms" / "datasketches_cms", 2048, pdir, "ds_cms_2k")),
    }


def panel_cs_2k() -> dict[str, list[float]]:
    pdir = RAW_DIR / "cs_2k"
    cfg = "rows=5 cols=2048"
    return {
        "sketchlib (fixed matrix)": read_throughputs(cli_bench("countsketch", "lib-fixedmatrix-fast", cfg, pdir)),
        "oxide":                    read_throughputs(cli_bench("countsketch", "oxide", cfg, pdir)),
        # final_cs is hard-coded 5x2048; --k is unused for InsertOpt CountSketch
        "InsertOptimized (Final)":  read_throughputs(cpp_bench(CPP_BUILD / "cs" / "final_cs", None, pdir, "final_cs_2k")),
    }


def panel_cms_cs_32k() -> tuple[list[str], dict[str, list[float]]]:
    """Returns (group_order, label->throughputs).
    Group prefix tells the plot how to colour/cluster CMS vs CS bars on one axis."""
    pdir = RAW_DIR / "cms_cs_32k"
    cfg = "rows=5 cols=32768"
    data = {}
    # CMS half (4 bars)
    data[("CMS@32K", "sketchlib (fixed matrix)")] = read_throughputs(cli_bench("cms", "lib-fixedmatrix-fast-32k", cfg, pdir))
    data[("CMS@32K", "oxide")]                    = read_throughputs(cli_bench("cms", "oxide", cfg, pdir))
    data[("CMS@32K", "datasketches (Rust)")]      = read_throughputs(cli_bench("cms", "datasketches", cfg, pdir))
    data[("CMS@32K", "datasketches (C++)")]       = read_throughputs(cpp_bench(CPP_BUILD / "cms" / "datasketches_cms", 32768, pdir, "ds_cms_32k"))
    # CS half (2 bars only — datasketches has no CountSketch)
    data[("CS@32K",  "sketchlib (fixed matrix)")] = read_throughputs(cli_bench("countsketch", "lib-fixedmatrix-fast-32k", cfg, pdir))
    data[("CS@32K",  "oxide")]                    = read_throughputs(cli_bench("countsketch", "oxide", cfg, pdir))
    return data


def panel_hll() -> dict[str, list[float]]:
    pdir = RAW_DIR / "hll"
    cfg = "lg_k=14"
    return {
        "sketchlib (classic lib)": read_throughputs(cli_bench("hll", "lib", cfg, pdir)),
        "oxide":                   read_throughputs(cli_bench("hll", "oxide", cfg, pdir)),
        "datasketches (Rust)":     read_throughputs(cli_bench("hll", "datasketches", cfg, pdir)),
        "datasketches (C++)":      read_throughputs(cpp_bench(CPP_BUILD / "hll" / "datasketches_hll", 14, pdir, "ds_hll")),
    }


def panel_kll() -> dict[str, list[float]]:
    pdir = RAW_DIR / "kll"
    cfg = "k=200"
    return {
        "sketchlib":               read_throughputs(cli_bench("kll", "lib", cfg, pdir)),
        "oxide":                   read_throughputs(cli_bench("kll", "oxide", cfg, pdir)),
        "datasketches (C++)":      read_throughputs(cpp_bench(CPP_BUILD / "kll" / "datasketches_kll", 200, pdir, "ds_kll")),
        "InsertOptimized (Final)": read_throughputs(cpp_bench(CPP_BUILD / "kll" / "final_kll", 200, pdir, "final_kll")),
    }


# ---------- Plotting ----------

def fmt_mops(v: float, _pos=None) -> str:
    return f"{v / 1e6:.0f}M"


def _bar_with_mini_box(ax, positions: list[float], values: list[list[float]], colors: list) -> None:
    """Draw a solid bar (0..mean) per impl with a narrow mini-boxplot of the runs
    overlayed on top to show run-to-run dispersion."""
    means = [sum(v) / len(v) for v in values]
    ax.bar(positions, means, width=0.62, color=colors, alpha=0.55, edgecolor="black", linewidth=0.6, zorder=1)
    bp = ax.boxplot(values, positions=positions, widths=0.20, showfliers=False,
                    patch_artist=True, zorder=3,
                    medianprops=dict(color="black", linewidth=1.2),
                    whiskerprops=dict(color="black", linewidth=0.8),
                    capprops=dict(color="black", linewidth=0.8),
                    boxprops=dict(linewidth=0.8))
    for patch in bp["boxes"]:
        patch.set_facecolor("white")
        patch.set_alpha(0.95)


def boxplot(data: dict[str, list[float]], title: str, out_png: Path, ylabel: str = "Throughput (items/sec)") -> None:
    labels = list(data.keys())
    values = [data[k] for k in labels]
    fig, ax = plt.subplots(figsize=(max(6, 1.4 * len(labels) + 2), 4.5))
    positions = list(range(1, len(labels) + 1))
    cmap = plt.get_cmap("tab10")
    colors = [cmap(i % 10) for i in range(len(labels))]
    _bar_with_mini_box(ax, positions, values, colors)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels)
    ax.set_title(title)
    ax.set_ylabel(ylabel)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_mops))
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    plt.setp(ax.get_xticklabels(), rotation=18, ha="right")
    fig.tight_layout()
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def boxplot_grouped(data: dict[tuple[str, str], list[float]], title: str, out_png: Path) -> None:
    """For the CMS+CS@32K combined panel: bars grouped by algorithm with a gap between groups."""
    # Preserve insertion order; identify group boundaries.
    keys = list(data.keys())
    positions = []
    pos = 1.0
    last_group = None
    for grp, _ in keys:
        if last_group is not None and grp != last_group:
            pos += 1.0  # extra gap between groups
        positions.append(pos)
        pos += 1.0
        last_group = grp
    values = [data[k] for k in keys]
    labels = [f"{lab}" for _, lab in keys]
    fig, ax = plt.subplots(figsize=(max(8, 1.2 * len(keys) + 2), 5.0))
    # Colour by impl name (identical impls across groups share colour).
    impl_to_color = {}
    cmap = plt.get_cmap("tab10")
    colors = []
    for _, lab in keys:
        if lab not in impl_to_color:
            impl_to_color[lab] = cmap(len(impl_to_color) % 10)
        colors.append(impl_to_color[lab])
    _bar_with_mini_box(ax, positions, values, colors)
    ax.set_xticks(positions)
    ax.set_xticklabels(labels, rotation=18, ha="right")
    # Group-name annotations under each cluster.
    group_centers: dict[str, list[float]] = {}
    for (grp, _), p in zip(keys, positions):
        group_centers.setdefault(grp, []).append(p)
    for grp, ps in group_centers.items():
        cx = sum(ps) / len(ps)
        ax.annotate(grp, xy=(cx, 0), xytext=(0, -78), textcoords="offset points",
                    ha="center", fontsize=12, fontweight="bold",
                    xycoords=("data", "axes fraction"))
    ax.set_title(title)
    ax.set_ylabel("Throughput (items/sec)")
    ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_mops))
    ax.grid(axis="y", linestyle=":", alpha=0.6)
    fig.tight_layout()
    fig.subplots_adjust(bottom=0.30)
    fig.savefig(out_png, dpi=160)
    plt.close(fig)
    print(f"  wrote {out_png}")


def dump_long_csv(panel: str, data) -> None:
    """Persist tidy-format data so plots are reproducible without re-running benches."""
    p = OUT_DIR / f"{panel}.csv"
    with p.open("w", newline="") as f:
        w = csv.writer(f)
        if isinstance(next(iter(data.keys())), tuple):
            w.writerow(["group", "impl", "run", "throughput_items_per_sec"])
            for (grp, lab), vals in data.items():
                for i, v in enumerate(vals, 1):
                    w.writerow([grp, lab, i, v])
        else:
            w.writerow(["impl", "run", "throughput_items_per_sec"])
            for lab, vals in data.items():
                for i, v in enumerate(vals, 1):
                    w.writerow([lab, i, v])
    print(f"  wrote {p}")


def main() -> int:
    only = set(sys.argv[1:]) or None

    panels: list[tuple[str, str, callable, callable]] = [
        ("cms_2k",      "CMS @ 5×2048 (Zipf s=1.1, 10M items)",                              panel_cms_2k,     boxplot),
        ("cs_2k",       "CountSketch @ 5×2048 (Zipf s=1.1, 10M items)",                       panel_cs_2k,      boxplot),
        ("cms_cs_32k",  "CMS vs CountSketch @ 5×32768 (Zipf s=1.1, 10M items)",               panel_cms_cs_32k, boxplot_grouped),
        ("hll",         "HLL @ lg_k=14 (Zipf s=1.1, 10M items)",                              panel_hll,        boxplot),
        ("kll",         "KLL @ k=200 (Zipf s=1.1, 10M items)",                                panel_kll,        boxplot),
    ]

    for tag, title, gen, plot in panels:
        if only and tag not in only:
            continue
        print(f"=== panel: {tag} ===", flush=True)
        data = gen()
        dump_long_csv(tag, data)
        plot(data, title, OUT_DIR / f"{tag}.png")

    return 0


if __name__ == "__main__":
    sys.exit(main())
