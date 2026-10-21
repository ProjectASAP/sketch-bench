# e2e: ASAPQuery-backend vs Prometheus

A first end-to-end comparison: one PromQL query over one synthetic dataset,
answered by [ASAPQuery-backend](https://github.com/ProjectASAP/ASAPQuery-backend)
(which executes ASAPPlanner's plan) and by Prometheus, with both systems fed
the same samples over Remote Write and queried through the same Prometheus
HTTP API at the same evaluation times. It is not a benchmark suite.

## Run

```sh
e2e/run.sh                 # setup + comparison; results in e2e/results/run-<UTC time>/
e2e/run.sh e2e/results/x   # same, into a chosen directory
```

`run.sh` calls `setup.sh`, which is idempotent:

1. downloads the pinned Prometheus release tarball into `e2e/.cache/` and
   checks its SHA-256;
2. fetches ASAPQuery-backend at the pinned commit into
   `e2e/.cache/ASAPQuery-backend`, refuses it if its `Cargo.toml` pins any
   ASAPPlanner rev other than the pinned one, and builds `control_plane` and
   `data_plane` (`cargo build --release --locked`);
3. builds the harness, `aqpbm-e2e` (workspace crate `e2e/harness`).

Everything builds into the workspace's one `target/` with
`CARGO_INCREMENTAL=0` and `CARGO_PROFILE_RELEASE_DEBUG=0`. A cold setup
downloads about 110 MB (the Prometheus tarball) plus a shallow backend
fetch, and adds about 1.4 GB to `target/`. A run takes about
20 s. Needs Rust, git, curl, and Linux on x86-64; no Docker.

Extra flags after the output directory go to `aqpbm-e2e`, for example
`--evaluations 12 --repeats 20 --seed 7`. See `aqpbm-e2e --help`.

## Pinned

| What | Pin | Where |
|---|---|---|
| Prometheus | 3.15.0, linux-amd64 tarball, SHA-256 checked | `pins.env` |
| ASAPQuery-backend | `c1ec0c2d940930ef701c10bbb21be569192bfc3a` | `pins.env` |
| ASAPPlanner | `7565c47e30609d9571d8a9a372bf5e195754b2bb` (the backend's own pin, checked by `setup.sh`) | `pins.env` |

The harness links no ASAPPlanner crate. The planner runs only inside the
backend's binaries, built from the backend's lockfile.

## What one run does

- **Query.** `quantile_over_time(0.99, latency_ms[60m])`, evaluated every
  10 min, with a rank-error requirement of 0.01
  (`workloads/latency_p99_60m.json`, which is the backend's
  `examples/planner-layering-example4b.json` with `series_count` and
  `rows_per_second` set to match the dataset).
- **Data.** 16 series of `latency_ms` (`instance="host-000"` to `host-015`,
  `job="aqpbm-e2e"`), one sample per second for 2 h, 115,200 samples. Values
  come from `aqpbm-datagen` (one `f64` column, Uniform[1, 1000), seed 42).
  `aqpbm-datagen` makes value columns with no time axis or labels, so the
  harness puts row `i` on series `i % 16` at time `start + (i / 16) s`. The
  timestamps are placed in the recent past: the data plane closes panes no
  later than the wall clock and drops samples more than 5 min ahead of it.
- **Arms.** The control plane plans the workload twice. Each plan gets a fresh
  Prometheus and a fresh data plane, and both arms see the same samples and
  evaluation times.
  - `query_time_raw`: default `DeploymentCapabilities`. The planner picks P2:
    at query time the data plane reads the 60 min raw window from Prometheus
    and builds a KLL sketch (k = 361) from it.
  - `ingestion_panes`: `{"raw_data_retained": false}`
    (`capabilities/no_raw.json`). The planner picks P3-m1: one KLL per series
    per 10 min pane, built at ingestion, with six panes merged at query time.
- **Replay.** Samples are sent in event-time order, 60 s of samples per
  Remote Write request, first to Prometheus and then to the data plane. When
  the replay reaches an evaluation time `T`, the harness calls
  `POST /api/v1/precompute/watermark?time_ms=T`, so the pane ending at `T`
  closes. Then it sends the query to each system 5 times at `T`. The data plane
  never builds the first pane it sees, so the first evaluation is at
  T0+70m, followed by 5 more 10 min apart.
- **The data plane runs with `--no-forward`.** An answer from it is the
  plan's own answer (`x-asap-execution: plan`), never Prometheus's. The run
  fails if any ASAP query is answered any other way.

## What is measured

Per arm, evaluation time and series (`answers.csv`):
- `prom_value` and `asap_value`, the two answers (from the first of the
  repeated requests);
- `truth`, the exact Prometheus-style quantile (linear interpolation at rank
  `q * (n - 1)`), computed by the harness from the generated samples in
  `(T - 60m, T]`;
- `rel_error = |asap - prom| / |prom|`;
- `asap_rank_error`: how far `q` lies from the normalized ranks the ASAP value
  holds in the true window. This is the planner's own accuracy metric.

Per arm and evaluation time (`queries.csv`):
- HTTP status and `x-asap-execution`;
- series counts;
- max and mean relative error;
- max rank error;
- `prom_vs_truth_max_abs`, which checks that Prometheus holds exactly the
  generated data;
- query latency on each side: the first request and the median of the 5,
  timed by the client from request sent to body read (`aqpbm-core`
  `WallClock`);
- watermark call latency.

Per arm (`summary.json`):
- control-plane wall time and peak RSS (from `wait4`);
- the selected plan;
- startup time of each process;
- Remote Write request count and summed round-trip time per system;
- RSS and CPU for each process, read from `/proc/<pid>` at idle (after
  startup), after each evaluation and at the end;
- the data plane's `/api/v1/status/asap` counters and Prometheus's
  `/api/v1/status/tsdb`;
- the pins, the dataset description and the timeline.

## Results of one run (2026-10-21, this machine: 40 cores, 251 GB RAM)

All 12 ASAP answers came from the plan (`200`, `x-asap-execution: plan`), and
both systems returned all 16 series each time. Prometheus matched the
harness's exact quantile at every point (`prom_vs_truth_max_abs = 0`). In
`ingestion_panes`, the data plane built 11 panes. It skipped the 9,600
samples of the first pane and dropped no samples as late.

| arm | eval time | ASAP exec | series P/A | max rel err | mean rel err | max rank err | Prom p50 ms | ASAP p50 ms |
|---|---|---|---|---|---|---|---|---|
| query_time_raw | T0+70m | 200 plan | 16/16 | 0.00640 | 0.00176 | 0.00528 | 36.41 | 115.30 |
| query_time_raw | T0+80m | 200 plan | 16/16 | 0.00370 | 0.00110 | 0.00333 | 35.73 | 115.22 |
| query_time_raw | T0+90m | 200 plan | 16/16 | 0.00265 | 0.00132 | 0.00222 | 34.70 | 108.56 |
| query_time_raw | T0+100m | 200 plan | 16/16 | 0.00353 | 0.00122 | 0.00278 | 35.32 | 110.58 |
| query_time_raw | T0+110m | 200 plan | 16/16 | 0.00380 | 0.00132 | 0.00361 | 35.33 | 109.84 |
| query_time_raw | T0+120m | 200 plan | 16/16 | 0.00633 | 0.00208 | 0.00444 | 35.22 | 109.98 |
| ingestion_panes | T0+70m | 200 plan | 16/16 | 0.00320 | 0.00134 | 0.00333 | 36.57 | 4.29 |
| ingestion_panes | T0+80m | 200 plan | 16/16 | 0.00395 | 0.00169 | 0.00389 | 34.69 | 4.27 |
| ingestion_panes | T0+90m | 200 plan | 16/16 | 0.00329 | 0.00140 | 0.00278 | 35.63 | 4.33 |
| ingestion_panes | T0+100m | 200 plan | 16/16 | 0.00305 | 0.00090 | 0.00222 | 34.56 | 4.32 |
| ingestion_panes | T0+110m | 200 plan | 16/16 | 0.00454 | 0.00164 | 0.00333 | 34.49 | 4.30 |
| ingestion_panes | T0+120m | 200 plan | 16/16 | 0.00403 | 0.00166 | 0.00278 | 34.62 | 4.30 |

| arm | plan | plan ms | CP peak RSS MB | Prom RSS idle/peak MB | DP RSS idle/peak MB | Prom CPU s | DP CPU s | Prom write ms | DP write ms |
|---|---|---|---|---|---|---|---|---|---|
| query_time_raw | P2 | 5.6 | 8.8 | 100.2 / 110.1 | 11.2 / 44.3 | 4.44 | 2.26 | 174.0 | 49.3 |
| ingestion_panes | P3-m1 | 6.3 | 8.8 | 101.4 / 101.4 | 11.5 / 14.6 | 2.58 | 0.19 | 173.6 | 48.0 |

`curl` against the same Prometheus gave the same 35 to 40 ms query time, so
that number is not an artifact of the harness's HTTP client.

## Known limits

- **One query, one small dataset.** 16 series and 2 h of data. The numbers
  say the pipeline works and show the order of magnitude, nothing more.
  Prometheus's latency at this size is mostly fixed cost.
- **ASAP answers are not bit-reproducible.** The samples and evaluation
  times are deterministic up to the wall-clock offset, but KLL compaction is
  randomized, so error values change from run to run. Across the runs made
  while building this, the max rank error stayed at or below 0.006, within
  the required 0.01.
- **The data plane's Remote Write time is not its ingestion time.** It
  answers 204 once the samples are queued for its engine. Its pane building
  finishes before the watermark call returns (`watermark_ms` in
  `queries.csv`, about 0.3 ms here). Prometheus's write time includes the
  TSDB append.
- **CPU and RSS numbers cover the whole process.** CPU is `utime + stime`
  over the process's life, startup and WAL setup included. The peak RSS is
  the larger of the final `VmHWM` and every `VmRSS` reading the harness took.
  `VmHWM` alone can read lower than an earlier `VmRSS` on these
  multi-threaded processes, and the readings alone miss short spikes. Both
  raw readings are in `summary.json`. Prometheus's RSS is mostly its
  baseline at this data size.
- **The arms run one after the other on the same machine.** They do not
  overlap, but the latencies were measured on a host with other load and
  there is no CPU pinning.
- **The query-time arm still depends on Prometheus.** In `query_time_raw`,
  ASAP's latency includes reading 57,600 raw samples from Prometheus as JSON.
- **Only instant queries are compared.** The data plane does not support
  range queries yet.
- **The replay is single-threaded and runs at full speed, not in real
  time.** Event time comes from the samples. The harness picks timestamps in
  the recent past because the data plane caps pane closing at the wall clock.
- **Linux on x86-64 only** (the Prometheus tarball, and `/proc` for resource
  numbers).

## Files

- `run.sh`, `setup.sh`, `pins.env`: entry points and pins.
- `workloads/latency_p99_60m.json`: the PlanningWorkload.
- `capabilities/no_raw.json`: DeploymentCapabilities for `ingestion_panes`.
- `harness/`: the `aqpbm-e2e` crate. It includes its own Remote Write v1
  sender (prost + snap, so it does not link the code under test), the dataset
  and exact answers, process management, and query timing.
- `.cache/` (gitignored): the Prometheus release and the backend checkout.
- `results/` (gitignored): run outputs (`queries.csv`, `answers.csv`,
  `summary.json`, and per-arm `plan.json`, `plan.log`, `prometheus.log`,
  `data_plane.log`).
