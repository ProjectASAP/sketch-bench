# Synthetic workload: AutoSketch vs. ASAP

Results of ProjectASAP/ASAPQuery#777's synthetic workload grid (§6 "Workload
grid"): the dashboard default, the 10 templates, shared replicas {8, 64} and
strictness {loose, strict}, each solved by ASAP, PerQuery-CostAware and
AutoSketch-Adapted at both weight settings (CPU only; Fargate prices) and
the SLA grid {0.01, 0.1, 1, 10} ms and none.

**Pending:** the full run waits for the targeted saturation run at the
workload's data shape (θ = 1.1, K = 1e4, quantiles on the Zipf ranks;
#777 §6) and a cost re-export (#157). Results are committed after it.

## Reproduce

```sh
# Tables and plan.tsv (inputs as in rqe-optimizer/data/autosketch-eval/README.md,
# plus #140's synthetic_cardinalities curves and the cost table's exact rows).
python3 scripts/export_autosketch_eval_table.py --synthetic \
  --curves out_grid_1e7_cost/saturation_curve.csv synthetic_cardinalities/saturation_curve.csv \
  --curves-big out_1e9/saturation_curve.csv \
  --saturation out_grid_1e7_cost/saturation.csv synthetic_cardinalities/saturation.csv \
  --saturation-big out_1e9/saturation.csv \
  --merge-curves out_merge_topk/saturation_merge_curve.csv \
                 out_merge_quantile/saturation_merge_curve.csv \
                 synthetic_cardinalities/saturation_merge_curve.csv \
  --cost-records out_grid_1e7_cost/saturation_cost.jsonl \
  --exact-costs COSTS/rqe_atomic_costs.json \
  --out TABLES
scripts/run_autosketch_vs_asap_synthetic.sh TABLES run
scripts/run_autosketch_vs_asap_synthetic.sh TABLES plot
# ASAP's one-time profiling time (#777 §7)
scripts/profiling_time.py rqe-optimizer/data/profiling-time.json NAME=FILE.jsonl ...
```
