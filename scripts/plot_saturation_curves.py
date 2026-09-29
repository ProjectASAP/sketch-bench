"""Error vs N per data distribution, one figure per sketch, with theoretical bounds.

Usage: plot_saturation_curves.py out/saturation_curve.csv OUT_DIR  (needs matplotlib, pandas, numpy)
"""
import os
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import pandas as pd  # noqa: E402

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from study_saturation import n_saturation  # noqa: E402

curve = pd.read_csv(sys.argv[1])
outdir = sys.argv[2]

INK, INK2, MUTED, SURF, LINE, BAND = "#0b0b0b", "#52514e", "#898781", "#fcfcfb", "#2a78d6", "#cde2fb"
SAT = "#eb6834"
plt.rcParams.update({
    "font.size": 9, "axes.edgecolor": MUTED, "axes.labelcolor": INK2,
    "xtick.color": MUTED, "ytick.color": MUTED, "axes.titlecolor": INK,
    "axes.spines.top": False, "axes.spines.right": False,
    "axes.grid": True, "grid.color": "#e8e7e3", "grid.linewidth": 0.6,
    "figure.facecolor": SURF, "axes.facecolor": SURF,
})
NAMES = {
    "cms-fastpath-vector2d": "CMS (rows=3 cols=1024) · error = are_top100",
    "countsketch-fastpath-vector2d": "CountSketch (rows=3 cols=1024) · error = are_top100",
    "cms-heap-topk-fastpath-vector2d": "CMS-heap top-k (rows=3 cols=1024) · precision@k (higher is better)",
    "hll": "HLL (lg_k=12) · relative error",
    "kll-percall": "KLL (k=200) · mean rank error",
    "dd": "DDSketch (alpha=0.01) · mean rank error",
}
KLAB = {1000: "1K", 100000: "100K", 10000000: "10M"}
BOUND = "#52514e"

import math  # noqa: E402
import numpy as np  # noqa: E402

W, M_HLL, K_KLL, ALPHA_DD, TOPK = 1024, 2 ** 12, 200, 0.01, 32
EPS_CMS = math.e / W          # CMS: |est - f| <= eps * N, prob >= 1 - e^-3
EPS_CS = math.sqrt(3 / W)     # CountSketch: |est - f| <= eps * ||f||_2


def zipf_p(theta, k):
    w = np.arange(1, int(k) + 1, dtype=float) ** (-theta)
    return w / w.sum()


def bound(sk, theta, k):
    """Theoretical error bound for one panel, in the metric's own units."""
    if sk.startswith("cms-heap-topk"):
        p = zipf_p(theta, k)
        kk = min(TOPK, len(p) - 1)
        return float(np.sum(p[:kk] - p[kk] > EPS_CMS)) / kk
    if sk.startswith("cms"):
        p = zipf_p(theta, k)[:100]
        return float(np.mean(EPS_CMS / p))
    if sk.startswith("countsketch"):
        p = zipf_p(theta, k)
        l2 = math.sqrt(float(np.sum(p ** 2)))
        return float(np.mean(EPS_CS * l2 / p[:100]))
    if sk == "hll":
        return 2 * 1.04 / math.sqrt(M_HLL)
    if sk.startswith("kll"):
        return 2.296 / K_KLL ** 0.9723
    if sk == "dd":
        return ALPHA_DD * theta / 2   # theta is the Pareto alpha here
    return None


BOUND_TEXT = {
    "cms-fastpath-vector2d": "bound: mean over top-100 of ε/p_i, ε=e/w (|f̂−f| ≤ εN)  →  N-independent; depends on θ, K",
    "countsketch-fastpath-vector2d": "bound: mean over top-100 of ε‖p‖₂/p_i, ε=√(3/w) (|f̂−f| ≤ ε‖f‖₂)  →  N-independent; depends on θ, K",
    "cms-heap-topk-fastpath-vector2d": "lower bound on precision@32: share of top-32 keys with p_i − p_33 > e/w (ignores heap eviction)  →  N-independent",
    "hll": "bound: 2σ = 2·1.04/√m, m=4096 (3.25%)  →  independent of N; depends only on distinct count (smaller error below 2.5m distinct)",
    "kll-percall": "bound: normalized rank error ε ≈ 2.296/k^0.9723 = 1.33% (99% conf., single quantile)  →  N-independent; exact while N fits the sketch",
    "dd": "bound: mean rank error ≤ α_dd·a/2 (from value error α_dd=0.01 and Pareto density)  →  N-independent; depends on Pareto a",
}

for sk, name in NAMES.items():
    c = curve[curve.sketch == sk]
    if c.empty:
        continue
    pareto = (c.dist == "pareto").all()
    params = sorted(c.param.unique())
    ks = [None] if pareto else sorted(c.cardinality.dropna().unique())
    nr, nc = len(ks), len(params)
    fig, axes = plt.subplots(nr, nc, figsize=(2.9 * nc + 0.6, 2.5 * nr + 1.0),
                             sharex=True, sharey=False, squeeze=False, constrained_layout=True)
    for r, k in enumerate(ks):
        for j, p in enumerate(params):
            ax = axes[r, j]
            cc = c[(c.param == p) & ((c.cardinality == k) if k is not None else True)].sort_values("n")
            if cc.empty:
                ax.set_visible(False)
                continue
            logy = (cc.seed_mean_error > 0).all() and "topk" not in sk
            ns, err, se = cc.n.tolist(), cc.seed_mean_error.tolist(), cc.seed_se.tolist()
            floor = min(err) / 3
            lo = [max(e - 2 * s, floor) if logy else e - 2 * s for e, s in zip(err, se)]
            hi = [e + 2 * s for e, s in zip(err, se)]
            ax.fill_between(ns, lo, hi, color=BAND, lw=0)
            ax.plot(ns, err, color=LINE, lw=2, marker="o", ms=3.5)
            b = bound(sk, float(p), k)
            off = b is not None and not logy and b > 5 * max(max(hi), 1e-12)
            if b is not None and not off:
                ax.axhline(b, color=BOUND, lw=1.4, ls=":")
            nsat = n_saturation(ns, err, 0.10, 3, se)
            if nsat is not None:
                ax.axvline(nsat, color=SAT, lw=1.2, ls="--")
                ax.plot([nsat], [err[ns.index(nsat)]], "o", ms=8, color=SAT, mec=SURF, mew=2)
                tag = f"N_sat={nsat:.0e}".replace("e+0", "e")
            else:
                tag = "not saturated"
            head = f"α={p:g}" if pareto else f"θ={p:g}, K={KLAB.get(int(k), k)}"
            btxt = "" if b is None else f", bound={b:.3g}" + (" (off scale)" if off else "")
            ax.set_title(f"{head}\n{tag}{btxt}", loc="left", fontsize=8.5)
            ax.set_xscale("log")
            if logy:
                ax.set_yscale("log")
            if r == nr - 1:
                ax.set_xlabel("items inserted N")
            if j == 0:
                ax.set_ylabel("empirical error")
    import textwrap
    note = (f"{BOUND_TEXT[sk]}   (dotted gray = theoretical bound). Each panel has its own "
            f"y-scale (log when all errors > 0); line = mean of {10 if pareto else 3} seeds, band = ±2 SE, dashed = "
            "N_saturation (within max(10% of plateau, 2 SE) of the last-3 mean)")
    fig.suptitle(name + "\n" + "\n".join(textwrap.wrap(note, 34 * nc)),
                 x=0.01, ha="left", color=INK, fontsize=11)
    path = f"{outdir}/error_vs_N__{sk}.png"
    fig.savefig(path, dpi=110)
    plt.close(fig)
    print(path)
