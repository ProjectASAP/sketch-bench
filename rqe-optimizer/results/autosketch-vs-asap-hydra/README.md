# AutoSketch vs. ASAP with Hydra candidates and roll-ups

Results of ProjectASAP/ASAPQuery#777's synthetic evaluation (§6.3), rerun with
everything added since #138:

- the planner may serve a coarse RQE from a finer deployment by merging its
  groups (**roll-ups**, #190), and an **ablation** without them;
- **Hydra** grids (hydra-hll, hydra-univmon-cardinality, hydra-kll) are
  candidates for ASAP and PerQuery, with accuracy measured on the eval's own
  data (#193 design; #195–#202 measurement and optimizer);
- a **multi-grouping** template set (`http` and `flows`, templates 11–17, #196)
  next to the classic mixed set;
- the KLL footprint is asap_sketchlib's real allocation (#192), and the cost
  table is re-measured.

Cost is billed by use (sketch-bench `docs/rqe_optimizer_cost_model.md`):
`w_cpu · AUC(CPU) + w_mem · AUC(memory)`, at two weight settings (CPU only, in
vCPU; AWS Fargate's per-vCPU and per-GiB prices, in $/hour). Every RQE has a
p95 accuracy target (0.05 in its family's own metric). An RQE with a coverage
rule is held to the worse of its smallest and largest covered group
(sketch-bench#189). Hydra is held to its measured worst error over the covered
groups, the worst over N and seeds.

## Methods

| Method | What it may use |
|---|---|
| ASAP | the MILP over every RQE jointly: shared deployments, roll-ups, Hydra grids |
| ASAP (no roll-ups) | the same MILP, each RQE only at its own grouping (or a Hydra grid): the roll-up ablation |
| PerQuery-CostAware | the same MILP without sharing (one deployment per RQE, no roll-ups) |
| AutoSketch-Adapted | fixed configs chosen by memory (NSDI '24); no Hydra (#159) |

## Results

Cost of each method's cheapest plan (no latency bound), absolute:

| Workload | RQEs | Weights | ASAP | ASAP (no roll-ups) | PerQuery | AutoSketch | RQEs rolled up | Deployments (with / without roll-ups) |
|---|---|---|---|---|---|---|---|---|
| mixed (classic) | 50 | CPU (vCPU) | 3.54 | 3.54 | 10.5 | 3.72e3 | 0 | 8 / 8 |
| mixed (classic) | 50 | Fargate ($/h) | 0.230 | 0.230 | 0.752 | 176 | 0 | 8 / 8 |
| mixed, r = 8 | 92 | CPU | 3.66 | 3.66 | 18.5 | 4.46e3 | 0 | 8 / 8 |
| mixed, m = 8 | 400 | CPU | 28.3 | 28.3 | 84.1 | 2.97e4 | 0 | 64 / 64 |
| mixed, m = 16 | 800 | CPU | 56.6 | 56.6 | 168 | 5.95e4 | 0 | 128 / 128 |
| mixed, m = 16 | 800 | Fargate | 3.68 | 3.68 | 12.0 | 2.81e3 | 0 | 128 / 128 |
| mixed + multi-grouping | 104 | CPU | 3.79 | 3.94 | 11.3 | 3.72e3 | 42 | 14 / 30 |
| mixed + multi-grouping | 104 | Fargate | 0.245 | 0.257 | 0.799 | 176 | 42 | 14 / 30 |
| mixed + multi-grouping, r = 8 | 200 | CPU | 3.94 | 4.08 | 19.9 | 4.46e3 | 84 | 14 / 30 |
| mixed + multi-grouping, m = 8 | 832 | CPU | 30.4 | 31.5 | 90.3 | 2.98e4 | 336 | 112 / 240 |
| mixed + multi-grouping, m = 16 | 1664 | CPU | 60.7 | 63.0 | 181 | 5.96e4 | 672 | 224 / 480 |
| mixed + multi-grouping, m = 16 | 1664 | Fargate | 3.91 | 4.11 | 12.8 | 2.81e3 | 672 | 224 / 480 |

Every workload: 0 RQEs dropped, 0 sanity violations (ASAP ≤ ASAP without
roll-ups ≤ PerQuery, and ASAP ≤ AutoSketch). `summary_synthetic.md` has
every frontier point; `summary_sla.md` every SLA.

Planning time (CPU weights): ASAP (candidates + MILP) vs. AutoSketch (search
plus its measured benchmark, approxbench accuracy runs at 1e8 items):

| Workload | RQEs | ASAP | PerQuery | AutoSketch |
|---|---|---|---|---|
| mixed (classic) | 50 | 0.72 s | 0.64 s | 212 s |
| mixed, m = 16 | 800 | 13.7 s | 11.4 s | 3,394 s |
| mixed + multi-grouping | 104 | 1.05 s | 0.83 s | 481 s |
| mixed + multi-grouping, m = 16 | 1664 | 18.1 s | 13.9 s | 7,702 s |

### Roll-ups

On the multi-grouping set, ASAP serves 40% of RQEs (42 of 104, and the same
share at r = 8, m = 8, m = 16) from a finer deployment. That halves its
deployments (30 → 14; 480 → 224 at m = 16) but saves only **3.3–3.6% of cost
under CPU weights and 4.4–4.8% under Fargate**, at every point of the
cost–latency frontier (`fig_frontier.png`, `fig_rollup_ablation.png`). The
deployments roll-ups remove are small (HLL and DDSketch at coarse groupings,
a few KB per group); the cost is dominated by ingest and storage at the finest
groupings, which every plan needs. ASAP's ~3× advantage over PerQuery comes
from sharing deployments across RQEs at the same grouping, not from roll-ups.
On the classic set every stream has one grouping, so roll-ups never apply and
the two ASAP lines coincide.

### Hydra

Hydra grids are candidates for every eligible RQE, and many are eligible
(e.g. hydra-hll at W ≥ 4096 for flows at ≥ 5% coverage; hydra-kll by service
at W = 16384 for every group). **Neither ASAP nor PerQuery chooses a Hydra
grid in any workload, weight setting, bound or SLA** (`hydra_rqes = 0`
throughout). A Hydra insert fans out to every label subset in every row and
costs 1.9 µs per record (flows, 3 labels) to 5.1 µs (http, 4 labels), against
3.85 ns for a per-group HLL insert; at the eval's rates that is 8–11 vCPU of
ingest for one grid, against ASAP's 0.05–1.9 vCPU for the whole stream. The
only near case is flows under Fargate weights, where a Hydra-only plan costs
1.8× ASAP's (it uses 4.4× less memory). A check on real traces (Alibaba 2022,
Google 2011; not committed) agreed: Hydra meets 0.05 only for groups holding
≥ 1–5% of records, and per-group sketches that grow with n use less memory
than the grid at the W that needs.

hydra-univmon-cardinality reads ≈ 100% error at N ≥ 1e6 at every W
(8 layers × a small heap saturate at ~1e6 distinct values per cell), so it is
ruled out by accuracy; cardinality's Hydra candidate is hydra-hll.

## Figures

- `fig_frontier.png`: version 1, cost vs. query latency per workload and
  weight setting; frontiers swept over latency bounds (ASAP, ASAP without
  roll-ups, PerQuery as lines; AutoSketch, which ignores latency, as a point).
- `fig_cost_vs_sla.png`: version 2, each method's cost at batch-latency SLAs
  {100, 300, 1000, 3000, 10000} ms (AutoSketch: ● meets the SLA, × misses).
  On the multi-grouping set the tightest feasible bound is 490 ms (the
  per-(service, endpoint) queries), so the 100 and 300 ms SLAs have no plan.
- `fig_planning_time.png`: planning time vs. RQEs (multi-grouping set).
- `fig_rollup_ablation.png`: ASAP vs. ASAP without roll-ups, cheapest plan per
  workload, CPU and Fargate.

## Inputs

- `inputs/saturation/optimizer_cost/rqe_atomic_costs.json`: the cost table,
  56 rows, measured serially on idle clnode138 (`study_saturation.py --phase
  optimizer-cost --seeds 3 --hydra-variants hydra-hll,hydra-kll`), including
  Hydra rows per dataset (`measured_at.dataset`) and #192's KLL footprint.
- `inputs/saturation/hydra_saturation.csv`: the Hydra study (`--phase hydra`,
  #202), 6,779 runs on 4 CloudLab nodes; hydra-kll on `hydra_http_latency`
  rerun with per-service latency scales (×1–10 log-uniform), so groups differ
  in distribution as they do in practice.
- The accuracy curves (`out_grid_1e7_cost/`, `out_1e9/`) are the committed
  #138 study's, in `../autosketch-vs-asap-inputs/saturation/` (#194).
- `inputs/tables/`: the workloads, `export_autosketch_eval_table.py
  --synthetic` (classic and all × r ∈ {1, 8} × m ∈ {1, 8, 16}).

## Reproduce

```sh
cargo build --release -p aqpbm-cli -p rqe-optimizer --bins --examples
DIR=rqe-optimizer/results/autosketch-vs-asap-hydra
# saturation dir = the #138 curves + this cost table and Hydra study:
SAT=$(mktemp -d); cp -r rqe-optimizer/results/autosketch-vs-asap-inputs/saturation/out_* $SAT/
cp -r $DIR/inputs/saturation/* $SAT/
while IFS=$'\t' read -r table target result; do
  target/release/examples/autosketch_vs_asap synthetic --table $DIR/inputs/tables/$table \
      --target $target --runs 3 --no-chosen --saturation-dir $SAT --out $DIR/$result
done < $DIR/inputs/tables/plan.tsv
python3 scripts/autosketch_benchmark_time.py --binary target/release/approxbench \
    --out $DIR/autosketch-benchmark-times.json $DIR/synthetic-*-tp95.json
python3 scripts/plot_autosketch_vs_asap_synthetic.py $DIR
python3 scripts/plot_autosketch_vs_asap_sla.py $DIR $DIR/synthetic-*-tp95.json
```

Runs: one large table per CloudLab node (clnode109, 155, 178, 167; the small
ones serially on clnode138), Xeon E5-2683 v3, 56 cores, 2026-10-10, `--runs 3`,
sketch-bench `hydra/eval-integration` at 44ac160.

## Caveats

- The cost model charges a per-group sketch its insert only, not routing a
  sample to its group's sketch (~100 ns per record per deployment on the
  real traces). Adding it raises ASAP's flows ingest from 0.11 to ~1.3 vCPU,
  still far below a Hydra grid's 8.2; the Hydra conclusion holds.
- Per-group sketch memory is asap_sketchlib's full allocation from
  construction (#192); sketches that grow with n would favour per-group
  sketches further.
- Hydra datasets: no flows DDoS burst (datagen can't tie a source set to a
  subnet), uniform proto, smaller UnivMon cells (3×256, 8 layers, heap 64).
- Template 12 groups by (dst_subnet, proto) rather than (dst_subnet,
  dst_port): answering 1e6 groups every minute took ~49 s and set every
  plan's batch latency above the largest SLA.
