# Grid cost, exact-vs-sketch crossover, recommendations

From `--phase cost --n-max 1e7 --seeds 3 --cost-rows 3` (serial, idle machine)
and `--phase crossover`; see "Grid cost and crossover N*" in
`docs/saturation_study.md`.

- `cost_cpu_memory_grid.png` (`scripts/plot_saturation_cost.py`): per-item
  insert, per-fold merge and per-query CPU and `memory_bytes` at N = 1e7 vs
  theta / alpha, one config per sketch.
- `crossover_exact_vs_sketch.png` (`scripts/plot_saturation_crossover.py`):
  memory (top) and CPU to ingest N items and answer one query (bottom) for the
  exact polars baseline (solid, one line per K) and every sketch config
  (dashed), at theta = 1 / Pareto 1.5. The vertical gap is the saving.
- `crossover.csv`: N* per point (exact >= 10x / 100x the sketch from that N on).
- `recommendations.csv` (`scripts/recommend_config.py` on ASAPQuery#746's
  `skew_summary.csv`): smallest-memory config per query, range and sketch
  family that meets the query's accuracy target, with flags.
