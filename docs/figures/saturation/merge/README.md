# Accuracy after merging

Drawn by `scripts/plot_saturation_merge.py` from `saturation_merge_curve.csv`
(`--merge-shards-list 1,4,16,64`, first config per sketch, N up to 1e7,
3 seeds, theta 0.5/1.0/1.5 x K 1e3/1e5/1e7, Pareto alpha 1.1/2).

- `merge_error_vs_N__<sketch>.png`: one panel per distribution; m = 1 (blue,
  thick) is one sketch fed all N items, m = 4/16/64 split the same stream into
  m contiguous shards, build one sketch per shard and merge them. Bands are
  ±2 SE over seeds.
- `merge_ratio_at_nmax.png`: merged / single error at N = 1e7, one dot per
  distribution and shard count. 1.0 means merging is exact.

Reading: CMS, CountSketch, HLL and DDSketch sit exactly on 1.0 (the m lines
overlap). KLL drifts within seed noise up to m = 16 and is 5–16% higher at
m = 64. CMS-heap top-k is unchanged at K = 1e3 but loses 0.03–0.09 precision
at K >= 1e5 with theta >= 1, most visibly once N > 1e5 — a globally heavy key
can fall out of every shard's heap — so for top-k the single-sketch curve is
optimistic for merged windows.
