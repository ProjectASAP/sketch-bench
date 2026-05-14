# TODO — sketchlib-tool (this repo)

Track record against [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md).

## Landed

| Phase | Deliverable | Status |
|---|---|---|
| 2 | Workspace scaffold + `sketch-*` crate stubs | ✅ |
| 3 | `sketch-core`: `Sketch` trait, `Probe`, workloads, v1 JSONL schema | ✅ |
| 4 | `sketch-bench`: `BenchRunner`, metrics (wall / CPU / RSS / latency / throughput), accuracy comparators (freq / cardinality / quantile / topk), Welford aggregator | ✅ |
| 7-lite | `sketch-cli`: unified `sketchlib` binary with 21 sketch impls wired end-to-end via a `(family, impl)` dispatch table | ✅ |
| 6-lite | `sketch-runtime`: `Sampler` (disabled / every-n / time-window), `RuntimeSwitch`, stdout + file + noop exporters, compile-time feature gate (`--no-default-features` → ZST fallback), overhead microbench | ✅ |
| 7-sweep | `sketchlib bench` config sweep: typed `ParamSet` in sketch-core, 21 wrappers refactored to `::new(&params)`, per-family default grids, `--config 'k=v1,v2'` Cartesian override, optional `sketch_config` field in v1 JSONL, fixed-shape impls skipped with stderr log | ✅ |
| 8-tput | `--raw-csv DIR` (legacy long-format CSVs), `polars` impl per family, `lib-fastpath-parallel` ("octo") impl per family, per-call query capture (hll/kll/dd), `scripts/run_{throughput,accuracy}.sh` orchestrators, `throughput/<family>/rust/` + `accuracy/<statistic>/rust/` trees retired | ✅ |

## Deferred (explicitly out-of-scope for this first cut)

### `sketch-profile` (MERGE_PLAN Phase 5)

- `perf_event_open` hardware counters (L1/L2/LLC refs+misses, branch, TLB, IPC)
- `perf record` + flamegraph shell-out
- `cachegrind` shell-out
- `heaptrack` shell-out
- VTune callstack CSV analyzer (Rust port of asap-fusion `analyze_alloc_callstacks.py`)
- CI lint forbidding `sketch-profile` as a dep of `sketch-runtime`

**Why deferred**: `perf_event_open` needs elevated kernel perms + is Linux-only. Landing the library + bench story first unblocks the paper's accuracy-profile claim (ASAPQuery-backend #2) without cross-OS / privilege complications.

### `sketch-runtime` remaining pieces (MERGE_PLAN Phase 6)

Core landed — `Sampler`, `RuntimeSwitch`, stdout / file / noop
exporters, compile-time feature gate. Still open:

- **`PrometheusExporter`** — `/metrics` endpoint that scrapes
  the last window per probe
- **`GrpcExporter`** + `proto/feedback.proto` — streaming
  unary to ASAPController
- **Tightening** the `sample_every_n = power-of-2` hot path
  (replace modulo with bitwise AND) to bring measured overhead
  closer to the design-doc ≤1% target on real sketches
- **Additional bench config** exposing MEMORY + CPU for
  per-window emission (today `Sampler` only captures
  throughput + latency in the window record)

### C++ binaries → v1 JSONL migration (MERGE_PLAN Phase 8) — RETIRED

Done. `cpp-bench/{hll,cms,cs,kll}/` now host v1-JSONL binaries on the shared `cpp-bench/common/` runner; `cms32k` is reachable via `--k 32768` on the cms binary. The original `cpp-bench/legacy/` subtree has been deleted (recoverable from git history).

Visualization layer still consumes the legacy long-format CSVs that `sketchlib bench --raw-csv DIR` emits; a v1-JSONL reader can be added later if needed.

### Legacy Rust binaries (MERGE_PLAN Phase 8) — RETIRED

Done in the 8-tput row above. `throughput/<family>/rust/`,
`accuracy/<statistic>/rust/`, `throughput/polars_*/`,
`throughput/octo/rust/`, and `accuracy/{nitro,octo}/rust/` are
gone; source recoverable from git history. Plot scripts moved to
`visualization/plots/{throughput,accuracy}/` and still consume the
sketch-cli-emitted CSVs through `--raw-csv`. The per-family C++
harnesses have been ported onto `cpp-bench/common/` (now live at
`cpp-bench/{hll,cms,cs,kll}/`); the empty `throughput/`, `accuracy/`,
the original `legacy/` top-level directory, and `cpp-bench/legacy/`
have all been deleted.

### Downstream app integration (MERGE_PLAN Phase 9)

- DataCollector — wrap its sketches in `Probe`, stream to `sketch-runtime`
- ASAPQuery — same
- asap-fusion — same; retire its own `microbench/` scaffolding
- ASAPController — consume the gRPC feedback stream, diff live samples vs. baselines

Gated on `sketch-runtime` landing first.

### Minor polish (not paper-blocking)

- [ ] Add a `sketchlib workload generate|describe` subcommand (MERGE_PLAN Phase 7)
- [ ] Criterion microbench proving `Probe<_, NoopSink>` is a no-op (MERGE_PLAN Phase 4, last bullet)
- [ ] YAML sweep-matrix config loader (borrowed pattern from asap-fusion `experiments/configs/`) — superset of `bench-sweep`
- [ ] Retire the private warning on `WorkloadAny` (dispatch.rs) by making the type `pub(crate)` visible across the module boundary
