# ERP v1 benchmark methodology

ERP v1 measures sketch error, resident bytes, and atomic insert, merge, and
query CPU cost. It does not benchmark an end-to-end window policy: the planner
composes those atomic measurements from the number of updates, pane merges,
queries, retained panes, and shared materializations.

## Synthetic matrix

The reproducible v1 matrix crosses:

- distribution catalog: uniform; Zipf (the discrete power-law family) with
  exponent in `{0.8, 1.1, 1.4}`; normal parameter points; and versioned
  empirical/custom trace descriptors;
- key cardinality: `{1K, 10K, 100K}`;
- stream length: at least `max(1M, 100 * cardinality)` events;
- burst variant: no burst, or two seeded random intervals with 50% additional
  resampled traffic (`--burst-interval-rows`, `--burst-intervals 2`,
  `--burst-extra-fraction 0.5`);
- sketch parameter grid: every deployable parameter point for CMS,
  CountSketch, HLL, KLL, and DDSketch.

The minimum stream length is a saturation rule, not a claim that event count
is irrelevant. A profile records `benchmark_events`; the consumer rejects it
below its configured sufficiency threshold. Above the threshold, nearest-shape
matching uses log-cardinality plus the parameters defined by that distribution
family. Different families are never interpolated. A future continuous
`power_law` generator is represented as its own family (`alpha`, `minimum`),
not silently treated as Zipf; v1's Zipf rows already cover the usual discrete
power-law key-frequency workloads used by AutoSketch.

Example with a deterministic traffic burst:

```sh
approxbench sketchbench --variant cms --library oxide --config 'width=1024 depth=5' \
  --dataset zipf --zipf-s 1.1 --cardinality 10000 --size 1000000 --dtype i64 \
  --burst-interval-rows 100000 --burst-intervals 2 \
  --burst-extra-fraction 0.5 --seed 42 --operations insert,merge,query \
  --metrics throughput,cpu,memory,accuracy --runs 10 --warmup-runs 3 --flat
```

Raw JSONL is retained. `approxbench erp` converts it to the versioned ERP
artifact; no plotted point is copied from a smoke test.

## AutoSketch-versus-ASAPQuery experiment

The primary comparison is AutoSketch-PerQuery versus the ASAPQuery backend
using ASAPPlanner. ASAPPlanner-Analytical uses Planner's theoretical sizing and
analytical cost implementation; ASAPPlanner-ERP uses this measured catalog.
Both receive identical candidate implementations, accuracy and memory targets,
event rate, window/slide, retention, and recurrence. NoSharing and exact are
attribution/reference baselines.

Figure 1 v1 reports executed end-to-end update CPU, query latency, retained
memory, and error for those methods. The accompanying raw records contain the
selected sketch/configuration, Planner policy, workload, seed, and command
provenance. Analytical-versus-measured prediction error is supplemental model
diagnostics, not Figure 1.
