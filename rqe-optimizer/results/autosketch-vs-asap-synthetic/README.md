# Synthetic workload: AutoSketch vs. ASAP

Results of ProjectASAP/ASAPQuery#777's synthetic workload grid (§6 "Workload
grid"): the dashboard default, the 10 templates, shared replicas {8, 64} and
strictness {loose, strict}. Each is solved by ASAP, PerQuery-CostAware and
AutoSketch-Adapted at both weight settings (CPU only; Fargate prices) and
across the SLA grid {0.01, 0.1, 1, 10} ms and none. The trace workloads
alibaba_v2022 and google_2011 run with the same inputs
(`scripts/run_autosketch_vs_asap.sh`, results under
`rqe-optimizer/results/autosketch-vs-asap/`); boom is left out for now.

**Pending:** results are committed after the full run on the regenerated
study (below). `rqe-optimizer/data/profiling-time.json` still holds the
earlier runs' wall times; it is stale until refilled from the new runs.

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
