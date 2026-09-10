# ERP v1 benchmark methodology

ERP v1 measures sketch error, resident bytes, and atomic insert, merge, and
query CPU cost. It does not benchmark an end-to-end window policy: the planner
composes those atomic measurements from the number of updates, pane merges,
queries, retained panes, and shared materializations.

## Synthetic matrix

The reproducible v1 matrix crosses:

- distribution: uniform and Zipf `s` in `{0.8, 1.1, 1.4}`;
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
matching uses log-cardinality and Zipf exponent. Uniform and Zipf profiles are
never interpolated.

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

## Analytical-versus-empirical experiment

For each parameter point, the analytical baseline assigns operation cost from
the sketch's asymptotic work (for CMS/CountSketch, rows touched; memory is
width times depth times counter bytes). ERP uses measured nanoseconds per
insert/merge/query and measured resident bytes. Both models receive identical
candidate sets, accuracy target, memory constraint, event rate, window/slide,
retention, and query recurrence. Report selected configuration, constraint
violations, predicted CPU, measured CPU, prediction error, and planning time.

Figure 1 v1 plots predicted versus measured CPU for all points, with a diagonal
ideal line, and reports median absolute percentage error and Spearman rank
correlation. The accompanying CSV contains every plotted point and command
provenance.
