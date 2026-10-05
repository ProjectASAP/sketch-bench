# Accuracy after merging: full top-k and KLL/DDSketch grids

`--phase accuracy --merge-shards-list 1,4,16,64`, N up to 1e7:
- CMS-heap top-k: all 8 configs (cols 256–16384 x rows 3/5) x theta {0 … 2} x K {1e3, 1e5, 1e7}, 3 seeds;
- KLL k {50, 200, 800} and DDSketch alpha_dd {0.005, 0.01, 0.02, 0.05} x Pareto a {1.1, 1.5, 2, 3}, 10 seeds.

Run on an otherwise idle machine (accuracy only). Plots by
`scripts/plot_saturation_merge.py`: one `merge_error_vs_N__<sketch>__<config>.png`
per config (m = 1 is one sketch over all N items; m = 4/16/64 split the stream
into contiguous shards, sketch each, merge), and `merge_ratio_at_nmax.png`
(merged / single error at N = 1e7; precision for top-k, so < 1 is worse).

`recommendations_with_merge.csv`: `scripts/recommend_config.py` rerun with these
merge curves (`--merge-curves`), so top-k and KLL recommendations use the error
at the query's shard count m = S / x instead of the single-sketch curve.

Findings:
- **DDSketch**: merged error equals single-sketch error everywhere (ratio 1.000).
- **Top-k**: median merged/single precision is 1.00 for every config; losses
  (down to 0.60) are concentrated at large K and mid-to-high theta, where a
  globally heavy key can miss every shard's heap. Wider configs do not remove
  them (cols 16384 still has points at 0.60–0.80).
- **KLL**: merged error is higher, and more so for larger k: median ratio at
  N = 1e7 is ≈ 1.0–1.1 for k = 50 / 200 but 1.12–1.25 for k = 800 (up to 1.32).
  At intermediate N (1e4–1e5) merged KLL error is up to 3–4x the single sketch
  for k = 800 with m = 4, converging by 1e6–1e7.
- **Recommendations**: no row is flagged "single-sketch curve optimistic" any
  more (43 before). Six changed, e.g. p99 latency over 5m windows now needs
  KLL k = 200 instead of k = 50 to meet the 1% target after merging; top-k on
  the near-uniform baselines (per-node / per-machine) meets no target.
