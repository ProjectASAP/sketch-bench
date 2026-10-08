# Synthetic workload: AutoSketch vs. ASAP

Results of ProjectASAP/ASAPQuery#777's synthetic workload grid (§6 "Workload
grid"): the dashboard default, the 10 templates, shared replicas r = 8 (on
one metric) and m = 8 and 16 metrics (one dashboard per metric, 21 · m RQEs),
all at one accuracy level, p95: 95% in each family's own metric (rank,
relative value and relative error at most 0.05; top-k precision at least
0.95). The trace workloads use the same level. Each is solved by ASAP, PerQuery-CostAware and
AutoSketch-Adapted at both weight settings (CPU only; Fargate prices) and
across the SLA grid {0.003, 0.01, 0.03, 0.1, 0.3, 1, 3, 10} ms and none. The trace workloads
alibaba_v2022 and google_2011 run with the same inputs
(`scripts/run_autosketch_vs_asap.sh`, results under
`rqe-optimizer/results/autosketch-vs-asap/`); boom is left out for now.

## Results (p95, 2026-10-07)

Run on four identical CloudLab nodes (`machine-*.txt`: dashboard and all templates on clnode155, r = 8 on clnode178, m = 8 on clnode167, m = 16 on clnode138) over the asap_sketchlib 0.3.0
study (#174, #178–#186), one workload at a time. Files: `synthetic-*.json`
(raw), `summary_synthetic.md` (every weights and SLA), and the figures
`fig_objective_vs_latency.png`, `fig_objective_by_dimension.png` and
`fig_planning_time.png`, and `fig_cost_vs_total_latency.png` (every workload,
synthetic and traces: cost vs. the sum of the RQEs' estimated latencies across
the SLA grid, from `scripts/plot_autosketch_vs_asap_workloads.py`). The traces are in `../autosketch-vs-asap/`
(`traces-*.json`, `summary.md`, `fig1_objective.png`,
`fig4_objective_vs_sla.png`).

With no latency SLA, each method's objective (CPU only, in vCPU; Fargate prices, in $/hour):

| Workload | RQEs | ASAP | PerQuery-CostAware | AutoSketch-Adapted | Σ window/slide (ASAP, PerQuery, AutoSketch) | ASAP planning (s) |
|---|---|---|---|---|---|---|
| dashboard (default) | 21 | 0.767 vCPU; 0.072 $/h | 2.51 vCPU; 0.265 $/h | 911 vCPU; 37 $/h | 8, 24, 7578 | 0.16 |
| all templates | 50 | 3.32 vCPU; 0.221 $/h | 9.55 vCPU; 0.721 $/h | 3.41e+03 vCPU; 138 $/h | 12, 57, 20631 | 0.80 |
| dashboard, shared r = 8 | 52 | 2.35 vCPU; 0.342 $/h | 9.9 vCPU; 1.55 $/h | 6.52e+03 vCPU; 265 $/h | 33, 109, 54243 | 0.88 |
| dashboard, m = 8 metrics | 168 | 6.14 vCPU; 0.576 $/h | 20.1 vCPU; 2.12 $/h | 7.28e+03 vCPU; 296 $/h | 64, 192, 60624 | 1.26 |
| dashboard, m = 16 metrics | 336 | 12.3 vCPU; 1.15 $/h | 40.1 vCPU; 4.24 $/h | 1.46e+04 vCPU; 593 $/h | 128, 384, 121248 | 2.63 |
| traces, alibaba_v2022 | 51 | 2.31 vCPU; 0.303 $/h | 4.48 vCPU; 0.417 $/h | 71.8 vCPU; 3.14 $/h | 17, 51, 1122 | 0.05 |
| traces, google_2011 | 27 | 0.00484 vCPU; 0.000378 $/h | 0.014 vCPU; 0.000801 $/h | 0.0645 vCPU; 0.00283 $/h | 9, 27, 126 | 0.03 |

No RQE was dropped as unservable, no sanity check failed (ASAP and PerQuery meet
every SLA; ASAP ≤ PerQuery and ASAP ≤ AutoSketch where they apply), and every RQE's accuracy is read from a
measured curve (`accuracy_source`). AutoSketch-Adapted's gap is its window
adapter: a window as long as the query's lookback, sliding every `T`, puts
each item into window/slide overlapping sketches (Σ window/slide), up to
1440 for a 24 h lookback at 1 m. PerQuery-CostAware is the strong baseline.

AutoSketch-Adapted ignores the SLA but never violates one: it never merges, so each
RQE's latency is within 1.4% of the lowest any deployment reaches (equal for every
quantile, exact and trace RQE; top-k picks cols = 1024 at 0.00038 ms vs. 0.00037 ms),
and RQEs no method can meet are excluded for every method.

AutoSketch-Adapted benchmarks each probed config once per metric, on that metric's data,
so its benchmark time grows with m (540, 4320 and 8640 s at 60 s per probe for m = 1, 8, 16).

ASAP's one-time profiling (#777 §7) is `rqe-optimizer/data/profiling-time.json`:
0.77 h of runs over the whole study, which ran in parallel on five machines.

## Inputs

Costs and accuracy come from one saturation-study directory `DIR`, the same
inputs the planner reads (#174, #178, #179):
- `DIR/out_grid_1e7_cost/`: the accuracy grid to N = 1e7, including the
  cost table's shape (Zipf 1.1 over 1e4 keys, Pareto a = 2), with merge
  curves for the lossy sketches;
- `DIR/out_1e9/`: the targeted points past 1e7 that the workload reads;
- `DIR/optimizer_cost/rqe_atomic_costs.json`: the cost table.

The tables from `export_autosketch_eval_table.py --synthetic` hold only the
workload: RQEs, streams, targets and data shapes.

## Reproduce

```sh
cargo build -p aqpbm-cli --release
# Accuracy grid with merge curves (parallel), then the targeted 1e9 points.
python3 scripts/study_saturation.py --phase accuracy --n-max 1e7 --seeds 3 \
    --jobs 48 --merge-shards-list 1,4,16,64 --out DIR/out_grid_1e7_cost
python3 scripts/study_saturation.py --phase accuracy --n-max 1e9 --seeds 3 \
    --jobs 16 --points-from POINTS.csv --out DIR/out_1e9
# Cost table, serially on a quiet machine.
python3 scripts/study_saturation.py --phase optimizer-cost --out DIR/optimizer_cost

# Workload tables and plan.tsv, then the runs and the figures.
python3 scripts/export_autosketch_eval_table.py --synthetic --out TABLES
SATURATION_DIR=DIR scripts/run_autosketch_vs_asap_synthetic.sh TABLES run
scripts/run_autosketch_vs_asap_synthetic.sh TABLES plot
# Traces: alibaba_v2022 and google_2011 (rqe-optimizer/data/autosketch-eval/table.json).
scripts/run_autosketch_vs_asap.sh DIR

# ASAP's one-time profiling time (#777 §7): the wall time of the study runs.
scripts/profiling_time.py rqe-optimizer/data/profiling-time.json \
    grid=DIR/out_grid_1e7_cost/saturation_accuracy.jsonl \
    subset=DIR/out_1e9/saturation_accuracy.jsonl \
    cost=DIR/optimizer_cost/rqe_atomic_costs_raw.jsonl
```

`--merge-shards-list` and `POINTS.csv` follow the workloads' queries: the
merge counts their KLL deployments can reach (`L / x`), and the points whose
items per instance exceed 1e7. These are the synthetic workload's (Zipf 1.1
over 1e4 keys, Pareto a = 2) and alibaba's quantiles (Pareto a = 3, up to
1.6e8 items).
