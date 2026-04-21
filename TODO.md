# TODO — sketchlib-tool (this repo)

Track record against [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md).

## Landed

| Phase | Deliverable | Status |
|---|---|---|
| 2 | Workspace scaffold + `sketch-*` crate stubs | ✅ |
| 3 | `sketch-core`: `Sketch` trait, `Probe`, workloads, v1 JSONL schema | ✅ |
| 4 | `sketch-bench`: `BenchRunner`, metrics (wall / CPU / RSS / latency / throughput), accuracy comparators (freq / cardinality / quantile / topk), Welford aggregator | ✅ |
| 7-lite | `sketch-cli`: unified `sketchlib` binary with 21 sketch impls wired end-to-end via a `(family, impl)` dispatch table | ✅ |

## Deferred (explicitly out-of-scope for this first cut)

### `sketch-profile` (MERGE_PLAN Phase 5)

- `perf_event_open` hardware counters (L1/L2/LLC refs+misses, branch, TLB, IPC)
- `perf record` + flamegraph shell-out
- `cachegrind` shell-out
- `heaptrack` shell-out
- VTune callstack CSV analyzer (Rust port of asap-fusion `analyze_alloc_callstacks.py`)
- CI lint forbidding `sketch-profile` as a dep of `sketch-runtime`

**Why deferred**: `perf_event_open` needs elevated kernel perms + is Linux-only. Landing the library + bench story first unblocks the paper's accuracy-profile claim (ASAPQuery-backend #2) without cross-OS / privilege complications.

### `sketch-runtime` (MERGE_PLAN Phase 6)

- `Sampler::every_n(N)` + `Sampler::time_window(d)` implementing `MetricsSink`
- Exporters: stdout / file / prometheus / gRPC
- `proto/feedback.proto` — streaming schema for ASAPController channel
- Overhead microbench (≤1% at 1/1024 sampling)

**Why deferred**: the paper's "live samples feed controller" loop is separately tracked in the DataCollector repo (see `DataCollector/TODO.md` blockers #1-#2). We'll land `sketch-runtime` when the controller side is ready to consume the gRPC stream.

### C++ binaries → v1 JSONL migration (MERGE_PLAN Phase 8)

- Update `cpp/` harnesses to emit v1 JSONL directly (schema header shared via `sketch-core/schema/v1.json`)
- Visualization layer dual-read compatibility for one release, then drop legacy shapes

**Why deferred**: C++ parity is pure mechanical translation. The 21 Rust impls via `sketchlib` already cover the accuracy / throughput story for the paper.

### Legacy Rust binaries (MERGE_PLAN Phase 8)

The `rust/src/bin/*` files still work unchanged — they emit the old `total_nanoseconds` shape. Now that `sketchlib bench --sketch X --impl Y` covers the same matrix via the v1 schema, these can be retired. One binary per family deleted per follow-up PR keeps diffs reviewable.

### Downstream app integration (MERGE_PLAN Phase 9)

- DataCollector — wrap its sketches in `Probe`, stream to `sketch-runtime`
- ASAPQuery — same
- asap-fusion — same; retire its own `microbench/` scaffolding
- ASAPController — consume the gRPC feedback stream, diff live samples vs. baselines

Gated on `sketch-runtime` landing first.

### Minor polish (not paper-blocking)

- [ ] Add a `sketchlib workload generate|describe` subcommand (MERGE_PLAN Phase 7)
- [ ] Expose `heap-jemalloc` feature end-to-end (currently wired but not exercised in the CLI default profile)
- [ ] Criterion microbench proving `Probe<_, NoopSink>` is a no-op (MERGE_PLAN Phase 4, last bullet)
- [ ] YAML sweep-matrix config loader (borrowed pattern from asap-fusion `experiments/configs/`)
- [ ] Retire the private warning on `WorkloadAny` (dispatch.rs) by making the type `pub(crate)` visible across the module boundary
