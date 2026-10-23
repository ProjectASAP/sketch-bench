# aqpbm-planeval: ASAPQuery-backend vs Prometheus

End-to-end evaluation of ASAPQuery-backend, which runs ASAPPlanner's
selected plan, against Prometheus. Both systems get the same generated
samples over Remote Write and the same PromQL query through the Prometheus
HTTP API at the same evaluation times. The run compares their answers,
query latency, ingest and query throughput, and memory and CPU.

This is a first version: one query over one synthetic dataset.

## Run

```sh
aqpbm-planeval/run.sh                              # results in aqpbm-planeval/results/run-<UTC time>/
aqpbm-planeval/run.sh aqpbm-planeval/results/x     # into a chosen directory
aqpbm-planeval/run.sh OUT --series 600             # extra flags go to the planeval binary
PLANEVAL_LABELS=region:4,service:5,instance:10 aqpbm-planeval/run.sh   # another label hierarchy
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
- `--labels region:2,service:2,instance:4` sets the series' label hierarchy
  (`run.sh` passes `PLANEVAL_LABELS`, default that one);
- `--series N` sets the total number of series by rescaling the hierarchy's
  last level (N must be a multiple of the levels above it);
- `--counter-metric NAME` names the counter written next to the queried
  gauge (default `requests_total`; empty for none);
- `--evaluations`, `--repeats` and `--seed`;
- `--load-seconds` and repeated `--load-concurrency` set the load test.

The planner always sees a workload made to match the generated data: the
queried source's `series_count`, the counter as a second source with
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

- **Query.** `quantile_over_time(0.99, latency_ms[60m])`, evaluated every
  10 min, with a rank-error requirement of 0.01
  (`workloads/latency_p99_60m.json`).
- **Series and labels.** The series sit on a label hierarchy, by default
  `region:2,service:2,instance:4`: 2 regions, 2 services in each, 4
  instances in each service, 16 series. A label value names its level and
  its index across the level (`region-1`, `service-3`, `instance-12`), so
  every child has one parent and `by (region)` is a union of
  `by (region, service)`.
- **Data.** Each series carries two metrics, one sample per second each:
  - the gauge `latency_ms`, which the query reads, takes an `aqpbm-datagen`
    column (Uniform[1, 1000), seed 42) as it is;
  - the counter `requests_total` takes a second column (Uniform[0, 10),
    seed 43) as increments and accumulates them per series from zero, so it
    only grows, at about 5 per second.

  `aqpbm-datagen` makes value columns with no time axis or labels, so the
  harness puts row `i` on series `i % series` at time
  `origin + (i / series + 1) s`. The timestamps lie in the recent past,
  because the data plane closes panes no later than the wall clock.
- **Arms.** One per deployment configuration. Each arm plans the workload
  with the control plane, then starts a fresh Prometheus and a fresh data
  plane. Every arm sees the same samples and evaluation times.

  | Arm | Capabilities | Accuracy guarantees |
  |---|---|---|
  | `default` | default: raw data stays in Prometheus | required |
  | `default_ignore_guarantees` | default | ignored (`--ignore-accuracy-guarantees`) |
  | `no_raw` | `{"raw_data_retained": false}` (`capabilities/no_raw.json`) | required |
  | `no_raw_ignore_guarantees` | `{"raw_data_retained": false}` | ignored |

- **Replay.** Samples go out in event-time order, 60 s of samples per
  Remote Write request. Each segment up to the next evaluation time goes to
  Prometheus as one block, then to the data plane as one block. The harness
  then calls `POST /api/v1/precompute/watermark?time_ms=T` and queries both
  systems at `T`. The data plane runs with `--stream-start-ms` set to the
  origin, so it builds the first pane too, and the first evaluation is one
  window after the origin (T0+60m), followed by 5 more 10 min apart.
- **The data plane runs with `--no-forward`.** Every answer it gives is the
  plan's own (`x-asap-execution: plan`), never Prometheus's. The run fails
  if any ASAP query is answered any other way.

## What is measured

Per arm, evaluation time and series (`answers.csv`):
- `prom_value` and `asap_value`, the two answers;
- `truth`, the exact Prometheus-style quantile (linear interpolation at
  rank `q * (n - 1)`), computed by the harness from the generated samples;
- `rel_error = |asap - prom| / |prom|`;
- `asap_rank_error`: how far `q` lies from the normalized ranks the ASAP
  value holds in the true window, the planner's own accuracy metric.

Per arm and evaluation time (`queries.csv`): HTTP status and
`x-asap-execution`; series counts; max and mean relative error; max rank
error; `prom_vs_truth_max_abs`, which checks Prometheus against the
harness's exact answer; query latency on each side (the first of
`--repeats` requests and their median, timed by the client); and the
watermark call's latency.

Per arm and system, at the last evaluation time (`load.csv`): query
throughput from closed-loop load steps. In each step, that many clients
send the query back to back for `--load-seconds`; the step reports
successful queries per second and their p50 and p99 latency. The clients
run on the same machine as both systems.

Per arm (`summary.json`):
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
- the pins, the dataset and the timeline.

## Results of one run (2026-10-23, 40 cores, 251 GB RAM, 16 series)

Labels `region:2,service:2,instance:4`, gauge and counter, 211,200
samples per system. All 24 ASAP answers came from the plan, every series
of both systems was matched by its full label set, and Prometheus matched
the harness's exact quantile at every point. Errors are maxima over the six
evaluations; latencies are the median over them of each evaluation's
median single-client latency; QPS is with 16 clients.

| Arm | Plan | Sketch | Max rel err | Max rank err | Prom ms | ASAP ms | Prom QPS | ASAP QPS |
|---|---|---|---|---|---|---|---|---|
| `default` | P2 | KLL, k = 361, built at query time | 0.0058 | 0.0044 | 35.9 | 114.8 | 314 | 91 |
| `default_ignore_guarantees` | P2 | KLL, k = 361, built at query time | 0.0065 | 0.0042 | 35.2 | 113.2 | 315 | 87 |
| `no_raw` | P1-m1 | KLL, k = 361, six 10 min panes | 0.0049 | 0.0044 | 35.3 | 3.9 | 356 | 705 |
| `no_raw_ignore_guarantees` | P2-m1 | DDSketch, alpha = 0.01, six 10 min panes | 0.0098 | 0.0125 | 35.5 | 1.1 | 324 | 2,836 |

In Prometheus after the run, `count by (region)` and
`count by (region, service)` give 8 and 4 series per group, every service
sits under one region, `resets(requests_total[2h])` is 0, and
`rate(requests_total[10m])` lies between 4.7 and 5.1 per series.

With guarantees ignored and raw data not kept, the planner selects
DDSketch. DDSketch bounds the relative error of the value (alpha = 0.01),
not the rank error the query asks for, so it has no guarantee for this
query, and the control plane reports the query as accepted without one.
The measured relative error stays under 0.01 while the rank error exceeds
it. In exchange, that arm answered a query in about 1 ms and served about
2,800 queries per second with 16 clients, against about 700 for `no_raw`.

## Findings

- **The data plane answers queries on one engine thread.** Ingestion,
  watermarks and query evaluations are commands on one queue, so its query
  throughput stops rising with more clients and its latency grows instead.
- **A query-time plan over a large raw window is refused.** With 600 or more
  series, every query of the `default` arm gets `422`, `execution failed:
  execution memory limit exceeded`, because the data plane runs each plan
  with the executor's default 64 MiB limit and no flag changes it. Reported
  as [ASAPQuery-backend#848](https://github.com/ProjectASAP/ASAPQuery-backend/issues/848).
  The run exits non-zero when that happens, by design.

## Known limits

- **One query, one small dataset.** The numbers show the pipeline works and
  the order of magnitude, nothing more.
- **ASAP answers are not bit-reproducible.** Samples and evaluation times
  are deterministic up to the wall-clock offset, but sketch compaction is
  randomized.
- **The arms run one after the other on one machine**, with no CPU pinning,
  and the load-test clients share it.
- **The `default` arms still depend on Prometheus.** Their ASAP latency
  includes reading the raw window from Prometheus as JSON, and the data
  plane's ingest numbers mean little there because it stores nothing.
- **Only instant queries.** The data plane does not support range queries.
- **Every series has the same value and rate distribution.** Groups differ
  only by noise: `sum by (service) (rate(requests_total[10m]))` is about 20
  for every service, so a top-k over groups ranks near-ties.
- **The replay runs at full speed, not in real time.** Event time comes from
  the samples.
- **Linux on x86-64 only** (the Prometheus tarball, and `/proc`).

## Files

- `run.sh`, `setup.sh`, `pins.env`: entry points and pins.
- `workloads/latency_p99_60m.json`: the PlanningWorkload.
- `capabilities/no_raw.json`: DeploymentCapabilities of the `no_raw` arms.
- `src/`: the `planeval` binary. It includes its own Remote Write v1 sender
  (prost + snap, so it does not link the code under test), the dataset and
  exact answers, process management, and query timing.
- `examples/asapplanner_demo.rs`: ASAPPlanner on its own, planner and
  executor in process with no backend or Prometheus
  (`cargo run --release -p aqpbm-planeval --example asapplanner_demo`).
- `.cache/` (gitignored): the Prometheus release and the backend checkout.
- `results/` (gitignored): run outputs (`queries.csv`, `answers.csv`,
  `load.csv`, `summary.json`, and per-arm `plan.json`, `plan.log`,
  `prometheus.log`, `data_plane.log`).
