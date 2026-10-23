# aqpbm-planeval: ASAPQuery-backend vs Prometheus

End-to-end evaluation of ASAPQuery-backend, which runs ASAPPlanner's
selected plan, against Prometheus. Both systems get the same generated
samples over Remote Write and the same PromQL queries through the Prometheus
HTTP API at the same evaluation times. The run compares their answers,
query latency, ingest and query throughput, and memory and CPU.

One run takes a workload of queries, each with its own repeat interval. Per
deployment configuration it first plans every query alone, which tells
which queries the backend supports there, then plans the supported ones
jointly and runs them. Prometheus is the one reference: PromQL's semantics
are Prometheus's, and it answers from the raw samples with no
approximation. Each query is judged against its own accuracy requirement.

## Run

```sh
aqpbm-planeval/run.sh                              # results in aqpbm-planeval/results/run-<UTC time>/
aqpbm-planeval/run.sh aqpbm-planeval/results/x     # into a chosen directory
aqpbm-planeval/run.sh OUT --series 600             # extra flags go to the planeval binary
aqpbm-planeval/run.sh OUT --query q01 --query q06  # only some of the workload's queries
PLANEVAL_LABELS=region:4,service:5,instance:10 aqpbm-planeval/run.sh   # another label hierarchy
PLANEVAL_SCALE= aqpbm-planeval/run.sh              # every group with the same distribution
PLANEVAL_WORKLOAD=aqpbm-planeval/workloads/latency_p99_60m.json aqpbm-planeval/run.sh  # one query
```

`run.sh` calls `setup.sh`, which is idempotent:

1. downloads the pinned Prometheus release tarball into
   `aqpbm-planeval/.cache/` and checks its SHA-256;
2. fetches ASAPQuery-backend at the pinned commit into
   `aqpbm-planeval/.cache/ASAPQuery-backend`, refuses it if its `Cargo.toml`
   pins any ASAPPlanner rev other than the pinned one, and builds
   `control_plane` and `data_plane` (`cargo build --release --locked`);
3. builds the `planeval` binary of this crate.

Everything builds into the workspace's one `target/` with
`CARGO_INCREMENTAL=0` and debuginfo off. A cold setup downloads about
110 MB and adds about 1.4 GB to `target/`. Needs Rust, git, curl, and Linux
on x86-64; no Docker.

Useful flags (`planeval --help` lists all):
- `--labels region:2,service:4,instance:2` sets the series' label hierarchy
  (`run.sh` passes `PLANEVAL_LABELS`, default that one);
- `--series N` sets the total number of series by rescaling the hierarchy's
  last level (N must be a multiple of the levels above it);
- `--scale service:1,10` gives each group of a hierarchy level its own
  factor (`run.sh` passes `PLANEVAL_SCALE`, default that one; empty for
  none);
- `--joint-size N` (default 6) caps how many queries are planned together,
  and `--plan-timeout-seconds` (default 600) kills a control-plane run that
  takes longer;
- `--query qNN`, repeatable, keeps only those queries;
- `--counter-metric NAME` names the counter written next to the gauge
  (default `requests_total`; empty for none);
- `--evaluations`, `--repeats` and `--seed`;
- `--load-seconds` (default 3) and repeated `--load-concurrency` (default
  16) set the load test.

The planner always sees a workload made to match the generated data: the
gauge source's `series_count`, the counter as a second source with
`metric_type: counter`, and the arrival rate of both. It is written to the
output directory as `workload.json`.

## Pinned

| What | Pin | Where |
|---|---|---|
| Prometheus | 3.15.0, linux-amd64 tarball, SHA-256 checked | `pins.env` |
| ASAPQuery-backend | `1c6abea8ea341e8ee723a11abb0a2e68df15f317` | `pins.env` |
| ASAPPlanner | `05fb36e4e9ff0b8e2ff6d43685785117ec0b9225` (the backend's own pin, checked by `setup.sh`) | `pins.env` |

The `planeval` binary links no ASAPPlanner crate. The planner and its
executor run only inside the backend's binaries, built from the backend's
lockfile.

## What one run does

- **Queries.** `workloads/templates.json` holds ten template queries, each
  with its own interval and an accuracy requirement of 0.01 (rank error
  for quantiles, relative error for the rest). They are numbered `q01` to
  `q10` in file order:

  | Id | Query | Every |
  |---|---|---|
  | q01 | `quantile_over_time(0.99, latency_ms[60m])` | 10 min |
  | q02 | `quantile_over_time(0.5, latency_ms[10m])` | 1 min |
  | q03 | `max_over_time(latency_ms[10m])` | 1 min |
  | q04 | `avg_over_time(latency_ms[30m])` | 5 min |
  | q05 | `sum by (service) (rate(requests_total[5m]))` | 1 min |
  | q06 | `topk(3, sum by (service) (rate(requests_total[10m])))` | 5 min |
  | q07 | `increase(requests_total[60m])` | 10 min |
  | q08 | `sum by (region) (count_over_time(latency_ms[10m]))` | 5 min |
  | q09 | `max by (region) (max_over_time(latency_ms[30m]))` | 10 min |
  | q10 | `quantile by (region) (0.99, latency_ms)` | 1 min |

- **Series and labels.** The series sit on a label hierarchy, by default
  `region:2,service:4,instance:2`: 2 regions, 4 services in each, 2
  instances in each service, 16 series. A label value names its level and
  its index across the level (`region-1`, `service-5`, `instance-12`), so
  every child has one parent and `by (region)` is a union of
  `by (region, service)`.
- **Data.** Each series carries two metrics, one sample per second each:
  - the gauge `latency_ms` takes an `aqpbm-datagen` column (Uniform[1, 1000),
    seed 42);
  - the counter `requests_total` takes a second column (Uniform[0, 10),
    seed 43) as increments and accumulates them per series from zero, so it
    only grows, at about 5 per second before scaling.

  With `--scale service:1,10`, every service multiplies its gauge values
  and its counter increments by a factor of its own, log-uniform in
  [1, 10]. This is `aqpbm-datagen`'s `scale_by`
  (`ColumnSpec::scale_factor`), keyed by the series' `service` label rather
  than by a generated string column. The gauge and the counter have
  different seeds, so different factors; `summary.json` lists them under
  `dataset.scale`.

  `aqpbm-datagen` makes value columns with no time axis or labels, so the
  harness puts row `i` on series `i % series` at time
  `origin + (i / series + 1) s`. The timestamps lie in the recent past,
  because the data plane closes panes no later than the wall clock.
- **Timeline.** The origin is a multiple of every query's interval. Each
  query's first evaluation is the first time on its own grid one full range
  after the origin (one interval for a query without a range). The run
  ends `longest range + (evaluations - 1) × longest interval` after the
  origin, rounded up to the common grid, so every query's last evaluation
  is the same time. With the templates and `--evaluations 6` that is 110 min:
  q01 is evaluated 6 times and q10 110 times.
- **Arms.** One per deployment configuration. Every arm sees the same
  samples and evaluation times.

  | Arm | Capabilities | Accuracy guarantees |
  |---|---|---|
  | `default` | default: raw data stays in Prometheus | required |
  | `default_ignore_guarantees` | default | ignored (`--ignore-accuracy-guarantees`) |
  | `no_raw` | `{"raw_data_retained": false}` (`capabilities/no_raw.json`) | required |
  | `no_raw_ignore_guarantees` | `{"raw_data_retained": false}` | ignored |

- **Probe, then joint plans.** In each arm, the control plane first plans
  every query alone (`<arm>/probe/qNN.*`). A query it cannot plan is
  unsupported in that arm, with the control plane's reason. The supported
  queries are then planned together in batches of `--joint-size`, in
  workload order (`<arm>/b1`, `<arm>/b2`, ...). The control plane fails a
  workload if any one query has no executable plan, and its planning time
  grows fast with the number of queries, hence the probe and the batches.
- **One batch.** A fresh Prometheus and a fresh data plane, with the batch's
  joint plan. Samples go out in event-time order, 60 s of samples per
  Remote Write request. Each segment up to the next evaluation time of any
  of the batch's queries goes to Prometheus as one block, then to the data
  plane as one block. The harness then calls
  `POST /api/v1/precompute/watermark?time_ms=T` and queries both systems at
  `T` with every query due then. The data plane runs with
  `--stream-start-ms` set to the origin, so it builds the first pane too.
- **The data plane runs with `--no-forward`.** Every answer it gives is the
  plan's own (`x-asap-execution: plan`), never Prometheus's. The run fails
  if any ASAP query is answered any other way.

## What is measured

Accuracy is measured against Prometheus, series by series. Both systems'
output series are matched by their full label set, so a `by (service)`
aggregation or a `topk` is compared group by group. The harness does not
evaluate PromQL itself.

A quantile's requirement is a rank error, and Prometheus's answer, one
interpolated value, does not give ASAP's rank. So for the two quantile
shapes the harness asks Prometheus, at the same time, for the raw samples
the quantile is taken over (its *rank basis*), and measures ASAP's rank in
them:
- `quantile_over_time(q, X[W])`: `X[W]`, one multiset per series;
- `quantile [by|without (...)] (q, E)`: `E`, one multiset per group.

As a check, it recomputes Prometheus's quantile from those samples
(`prom_vs_raw_quantile_max_abs`); zero shows they are the samples
Prometheus's answer came from. The rank-basis request is not timed.

Each query is judged by its own requirement metric: rank error for `rank`,
relative error to Prometheus for `relative`, absolute error for `absolute`.
A query **meets** its requirement when the plan answered every evaluation,
with the same series as Prometheus, and no value's error exceeded
`max_error`. The planner's guarantee allows a failure probability, so one
value over the bound is a miss here even when the guarantee holds.

Per arm and query (`per_query.csv`, the one to read first):
- whether the query is supported alone, why not, and its planning time and
  method alone;
- its joint batch, the joint plan's method for it (the summaries under its
  root, such as `Kll(k=361)@ingestion` or `Exact(Rate)@query + Exact(Sum)@query`),
  whether any is a sketch, and the guarantee the planner states for it
  (`exact`, `rank<=0.0099 (p_fail 0.000625)`, ...), read from `plan.json`;
- evaluations, how many the plan answered, and series only Prometheus
  answered (`missing`) or only ASAP (`extra`). For a `topk` these say
  whether the two systems picked the same series;
- maximum and mean relative error, maximum absolute error, and for a
  quantile the maximum rank error in its rank basis;
- the requirement: ASAP's largest error in its metric, how many values were
  checked and how many exceeded `max_error`, and whether the query met it
  (`requirement_met`);
- the median over its evaluations of each evaluation's median latency on
  each side (`--repeats` requests per evaluation, timed by the client).

Per arm, query and evaluation time (`queries.csv`): HTTP status and
`x-asap-execution`; series counts; matched, missing and extra series;
errors; for a quantile, `prom_vs_raw_quantile_max_abs`; the requirement's
largest error, values checked and violations; the latencies; and the
watermark call's latency.

Per output series (`answers.csv`): both values and their errors, the error
in the requirement metric, and for a quantile the size of its multiset,
Prometheus's quantile recomputed from it, and ASAP's rank error in it.

Per arm and query alone (`probes.csv`): supported or not, the reason, the
planning time, and the method and guarantee of the query's plan alone.

Per arm, batch and query, at the last evaluation time (`load.csv`): query
throughput from closed-loop load steps. In each step, that many clients
send the query back to back for `--load-seconds`; the step reports
successful queries per second and their p50 and p99 latency. The clients
run on the same machine as both systems.

Per arm and batch (`summary.json`):
- control-plane wall time and peak RSS, and the selected plan;
- startup time of each process;
- ingest throughput: samples absorbed per second by one sequential writer.
  For Prometheus the time is its Remote Write round trips, which return
  after the TSDB append. For the data plane it is its round trips, which
  return once samples are queued, plus each segment's watermark call, which
  returns only after every queued sample is ingested and the panes up to
  `T` are built;
- RSS and CPU for each process from `/proc/<pid>`;
- the data plane's `/api/v1/status/asap` and Prometheus's
  `/api/v1/status/tsdb`;
- the pins, the dataset with its scale factors, and the timeline.

## Results of one run (2026-10-23, 40 cores, 251 GB RAM, 16 series)

Labels `region:2,service:4,instance:2`, services scaled in [1, 10], gauge
and counter, 211,200 samples per system and batch. Every query a joint plan
covered was answered by the plan at every evaluation, and no output series
was missing or extra on either side. Every requirement is 0.01: rank error
for the quantiles, relative error for the rest. "Max err" is ASAP's largest
error in that metric over all the query's evaluations and series. Latencies
are medians over its evaluations of each evaluation's median single-client
latency. Prometheus's latency is from the `default` arm (the others agree
within 0.3 ms).

| Query | Every | Metric | `default`: method | max err | met | ASAP ms | `no_raw`: method | max err | met | ASAP ms | `no_raw_ignore_guarantees`: method | max err | over / checked | met | ASAP ms | Prom ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| q01 | 10m | rank | KLL k=361, query time | 0.0058 | yes | 115.8 | KLL k=361, ingestion | 0.0044 | yes | 3.8 | DDSketch α=0.01, ingestion | 0.0117 | 1 / 96 | **no** | 1.1 | 35.8 |
| q02 | 1m | rank | KLL k=361, query time | 0.0017 | yes | 18.5 | KLL k=361, ingestion | 0.0017 | yes | 3.7 | DDSketch α=0.01, ingestion | 0.0150 | 29 / 1,616 | **no** | 1.3 | 5.8 |
| q03 | 1m | relative | exact Max, query time | 0 | yes | 0.9 | exact Max, ingestion | 0 | yes | 0.9 | same as `no_raw` | 0 | 0 / 1,616 | yes | 0.9 | 2.8 |
| q04 | 5m | relative | exact Sum, Count, query time | 0 | yes | 1.5 | exact Sum, Count, ingestion | 0 | yes | 1.5 | same as `no_raw` | 0 | 0 / 272 | yes | 1.5 | 7.1 |
| q05 | 1m | relative | exact Rate, Sum, query time | 0 | yes | 0.8 | exact Rate at ingestion, Sum at query | 0 | yes | 0.8 | same as `no_raw` | 0 | 0 / 848 | yes | 0.8 | 1.9 |
| q06 | 5m | relative | exact Rate, Sum, query time | 0 | yes | 0.8 | exact Rate at ingestion, Sum at query | 0 | yes | 0.8 | same as `no_raw` | 0 | 0 / 63 | yes | 0.8 | 2.9 |
| q07 | 10m | relative | exact Increase, query time | 0 | yes | 0.9 | exact Increase, ingestion | 0 | yes | 0.8 | same as `no_raw` | 0 | 0 / 96 | yes | 0.8 | 13.3 |
| q08 | 5m | relative | exact Count, Sum, query time | 0 | yes | 0.7 | exact Count at ingestion, Sum at query | 0 | yes | 0.7 | same as `no_raw` | 0 | 0 / 42 | yes | 0.7 | 2.8 |
| q09 | 10m | relative | exact Max, query time | 0 | yes | 0.7 | exact Max at ingestion and query | 0 | yes | 0.7 | same as `no_raw` | 0 | 0 / 18 | yes | 0.7 | 7.0 |
| q10 | 1m | rank | KLL k=361, query time | 0 | yes | 1.3 | unsupported | - | - | - | unsupported | - | - | - | - | 1.3 |

"0" for an exact plan is at most 3e-16, floating-point rounding.
`default_ignore_guarantees` selected the same methods as `default` and met
every requirement too. For every quantile, Prometheus's answer equals the
quantile of the raw samples it returned for the rank basis
(`prom_vs_raw_quantile_max_abs` is 0), so the rank errors are measured on
the samples Prometheus used. The joint plans:

| Arm | Batch | Queries | Plan time | Control-plane peak RSS |
|---|---|---|---|---|
| `default` | b1 | q01–q06 | 3.5 s | 58 MB |
| `default` | b2 | q07–q10 | 0.5 s | 27 MB |
| `no_raw` | b1 | q01–q06 | 0.1 s | 31 MB |
| `no_raw` | b2 | q07–q09 | 0.0 s | 34 MB |

With 16 clients, the data plane served q01 at 88 queries per second in
`default` and 701 in `no_raw`, against about 340 for Prometheus. It served
more than Prometheus on q07, q08 and q09 (q09: about 5,900 against 2,200).
On the other queries Prometheus served more (q03: 5,200 against 4,200;
q05: 8,300 against 4,500), although the data plane's single-client latency
is lower on every exact query.

## Findings

- **Every plan with a guarantee met its requirement, at every evaluation.**
  That is 9 queries in `no_raw` and 10 in `default`, sketches and exact
  plans alike.
- **Ten of ten queries are supported alone with raw data kept, nine of ten
  without.** `quantile by (region) (0.99, latency_ms)`, an aggregation over
  an instant vector, has only query-time candidates, so without raw data it
  is rejected (`RawDataNotRetained`).
- **Joint planning time grows fast with the number of queries.** Six
  queries took 3.5 s and four 0.5 s in `default`. All ten together were
  killed after 600 s (at 330 MB of RSS), so the run plans them in batches
  of six. Without raw data, all ten planned together fail in under a
  second, because q10 is unsupported there.
- **Every exact plan matches Prometheus exactly**, `rate` and `increase`
  included, so the backend's pane-merged rate follows Prometheus's
  extrapolation on this data. The `topk` picks the same three services as
  Prometheus at every evaluation.
- **A quantile's relative error to Prometheus says little about its
  requirement.** q02 (p50 over 10 min, 600 samples) has a rank error of at
  most 0.0017, one sample, yet a relative error of up to 0.020. q10's p99
  over 8 instant values per region differs from Prometheus by up to 0.049,
  yet its rank error is 0: KLL returns the largest of the 8 values, a
  correct 0.99 quantile by rank, while Prometheus interpolates between the
  two largest.
- **With guarantees ignored and raw data not kept, the planner selects
  DDSketch** for both `quantile_over_time` queries, and both miss their
  requirement. DDSketch bounds relative value error, not the rank error
  asked for: q01 exceeded 0.01 in 1 of 96 values (up to 0.0117) and q02 in
  29 of 1,616 (up to 0.0150), while their relative error to Prometheus
  stayed under 0.01.
- **A query-time sketch is slower than Prometheus; an ingestion-time one is
  faster.** q01 takes 116 ms with KLL built at query time from a 60 min raw
  window read from Prometheus, against 36 ms for Prometheus and 3.8 ms with
  KLL built at ingestion.
- **The data plane answers queries on one engine thread.** Ingestion,
  watermarks and query evaluations are commands on one queue, so its query
  throughput stops rising with more clients and its latency grows instead.
  With 16 clients, Prometheus serves more of most exact queries even where
  the data plane answers one client 2 to 3 times faster.
- **A query-time plan over a large raw window is refused.** With 600 or more
  series, every query of the `default` arm gets `422`, `execution failed:
  execution memory limit exceeded`, because the data plane runs each plan
  with the executor's default 64 MiB limit and no flag changes it. Reported
  as [ASAPQuery-backend#848](https://github.com/ProjectASAP/ASAPQuery-backend/issues/848).
  The run exits non-zero when that happens, by design.

## Known limits

- **Prometheus is the only reference.** If Prometheus and ASAP agree on a
  wrong answer, the run cannot tell. Rank error is measured only for the two
  quantile shapes above; another query whose requirement is a rank error is
  not judged.
- **Ten queries, one small dataset.** The numbers show the pipeline works and
  the order of magnitude, nothing more.
- **ASAP answers are not bit-reproducible.** Samples and evaluation times
  are deterministic up to the wall-clock offset, but sketch compaction is
  randomized.
- **The arms and batches run one after the other on one machine**, with no
  CPU pinning, and the load-test clients share it.
- **The `default` arms still depend on Prometheus.** Their ASAP latency
  includes reading the raw window from Prometheus as JSON, and the data
  plane's ingest numbers mean little there because it stores nothing.
- **Only instant queries.** The data plane does not support range queries.
- **The replay runs at full speed, not in real time.** Event time comes from
  the samples.
- **Linux on x86-64 only** (the Prometheus tarball, and `/proc`).

## Files

- `run.sh`, `setup.sh`, `pins.env`: entry points and pins.
- `workloads/templates.json`: the ten template queries (the default).
  `workloads/latency_p99_60m.json`: q01 alone.
- `capabilities/no_raw.json`: DeploymentCapabilities of the `no_raw` arms.
- `src/`: the `planeval` binary. It includes its own Remote Write v1 sender
  (prost + snap, so it does not link the code under test), the dataset,
  quantile and rank-error arithmetic, the workload's queries and their
  rank bases, what a plan does per query,
  process management, and query timing.
- `examples/asapplanner_demo.rs`: ASAPPlanner on its own, planner and
  executor in process with no backend or Prometheus
  (`cargo run --release -p aqpbm-planeval --example asapplanner_demo`).
- `.cache/` (gitignored): the Prometheus release and the backend checkout.
- `results/` (gitignored): run outputs (`per_query.csv`, `probes.csv`,
  `queries.csv`, `answers.csv`, `load.csv`, `summary.json`, the
  `workload.json` the planner saw, and per arm the probe plans and each
  batch's `workload.json`, `plan.json`, `plan.log`, `prometheus.log` and
  `data_plane.log`).
