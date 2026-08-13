# MMERGE_PLAN

Whoever needs this... check the commented source code....

Otherwise, ignore this.

<!-- # Merge Plan — `sketch-profiler` → `sketchlib-bench` (→ `sketchlib-tool`)

Turns two repos into one, under the design in [`DESIGN.md`](DESIGN.md).

- **Keeper repo**: `sketch-bench` (this repo). ~13k LOC Rust + C++, mature harness, visualization, multi-language. Ships as brand `sketchlib-tool` with CLI binary `sketchlib`.
- **Absorbed repo**: `sketch-profiler`. ~75 LOC Rust, prototype. Only real content is `src/data_gen/` (workload shape primitives) and a `sketchlib-rust` git submodule.

---

## Why merge into this repo (not the other way / not a new repo)

| Option | Cost | Benefit | Verdict |
|---|---|---|---|
| (a) Merge `sketch-profiler` → `sketchlib-bench` | Low: 75 LOC to relocate | Keep all history, viz, scripts, output conventions | **Chosen** |
| (b) Merge `sketchlib-bench` → `sketch-profiler` | High: ~13k LOC + C++ + CMake + viz to move, `Cargo.toml` to rebuild | None — destination is 1% the size | Rejected |
| (c) New third repo | Highest: two filter-repo migrations, duplicate CI, broken bookmarks | Only if neutrality mattered — it doesn't (these aren't peers) | Rejected |

---

## Migration phases

Each phase leaves the repo in a **working, mergeable state**. No long-lived branches.

### Phase 0 — Prep (no behavior change)

- [ ] Create `docs/DESIGN.md` and `docs/MERGE_PLAN.md` (this doc).
- [ ] Update `README.md` to point at `docs/DESIGN.md` and the migration state.
- [ ] Confirm downstream app owners (asap-fusion, DataCollector, ASAPQuery, ASAPController) have read the design.

### Phase 1 — Absorb `sketch-profiler`

Preserve `sketch-profiler` history via `git subtree`:

```bash
cd /home/zeying/repos/sketch-bench
git remote add sketch-profiler /home/zeying/repos/sketch-profiler
git fetch sketch-profiler
git subtree add --prefix=_import/sketch-profiler sketch-profiler/main --squash=false
```

Then relocate the valuable bits and drop the rest:

- [ ] Move `_import/sketch-profiler/src/data_gen/` → `sketch-core/src/workload/` (Phase 3 will create the crate; for now land as `workload/` under a temporary path and re-export).
- [ ] Discard `_import/sketch-profiler/src/main.rs` (stub `"Hello, world!"`) and empty `lib.rs`.
- [ ] Drop the `sketchlib-rust` submodule from the imported tree — this repo already pulls the sketch impls it needs via crates / local paths.
- [ ] Remove `_import/` once contents are relocated.
- [ ] On success, mark `sketch-profiler` repo archived on the host and add a README pointer in it to this repo.

### Phase 2 — Workspace scaffold

- [ ] Convert top-level `Cargo.toml` into a workspace with members: the new `sketch-*` crates (stubs at first) and the existing `rust/` crate. The Criterion micro-bench crate (`rust-microbench/`, formerly `benchmark/`) stays a standalone cargo project, not a workspace member.
- [ ] Create empty crates with stub `lib.rs`: `sketch-core`, `sketch-bench`, `sketch-profile`, `sketch-runtime`, `sketch-cli`.
- [ ] CI: `cargo check --workspace` and existing benches still build.

### Phase 3 — Populate `sketch-core`

- [ ] Define the `Sketch` trait (§4.1 of DESIGN).
- [ ] Define `Probe<S, Sink>` decorator (§4.2).
- [ ] Relocate workload types from Phase 1 into `sketch-core/src/workload/`.
- [ ] Port YAML config loader pattern from asap-fusion `experiments/configs/` (with `{SIZE}` expansion).
- [ ] Define `Report` serde types + v1 JSONL schema (§4.4).
- [ ] Unit test schema round-trip.

### Phase 4 — Populate `sketch-bench`

Target module layout: DESIGN §5.2. Target API: DESIGN §5.3.

- [ ] `src/config.rs` — `BenchConfig` + `MetricsMask` bitflags (THROUGHPUT | LATENCY | CPU | MEMORY | ACCURACY).
- [ ] `src/metrics/mod.rs` — `MetricsSink` trait + `NoopSink` (zero-cost) + `FullSink`.
- [ ] `src/metrics/time.rs` — `WallClock` (`Instant`) + `CpuTime` (`getrusage(RUSAGE_SELF)`); port timing primitives from the existing `rust-microbench/` crate (formerly `benchmark/`).
- [ ] `src/metrics/latency.rs` — `LatencyRecorder` using `hdrhistogram`; inert when `MetricsMask::LATENCY` unset.
- [ ] `src/metrics/memory.rs` — `Rss` from `/proc/self/status:VmHWM`; `JemallocPeak` via `tikv-jemalloc-ctl` behind the `heap-jemalloc` feature.
- [ ] `src/metrics/throughput.rs` — `ItemsPerSec` (insert + query phases separately).
- [ ] `src/aggregation/welford.rs` — numerically-stable online mean + variance.
- [ ] `src/aggregation/mod.rs` — `RunStats` (mean / stddev / 95% CI, z=1.96 normal approx).
- [ ] `src/accuracy/mod.rs` — `GroundTruth<S: Sketch>` trait; adopt the existing comparators under the top-level `accuracy/` dir.
- [ ] `src/accuracy/{frequency,cardinality,quantile,topk}.rs` — default impls per sketch family (CMS/CS, HLL, KLL, Top-k).
- [ ] `src/runner.rs` — `BenchRunner<S, W, G>` taking a fresh-sketch factory closure (`FnMut() -> S`), running warmup + N measured iterations, feeding `Probe<S, FullSink>`.
- [ ] `src/report.rs` — `BenchReport` + `per_run` + `aggregated`; serialize to the v1 JSONL schema in `sketch-core`.
- [ ] Cargo features: `heap-jemalloc` (off by default), `hdrhist` (on), `accuracy-topk` (on).
- [ ] `benches/self_overhead.rs` — criterion bench proving `Probe<_, NoopSink>` is a no-op.
- [ ] Wire one existing binary (`rust/src/bin/hll_oxide.rs`) end-to-end through `BenchRunner` as the proof-of-shape.

### Phase 5 — Populate `sketch-profile`

- [ ] Add `hw_counters/` using `perf-event2`: L1D/LLC refs + misses, branch instructions + misses, dTLB/iTLB misses, instructions + cycles → IPC.
- [ ] Port `sketchlib_kll_vtune.rs` harness from asap-fusion into `sketch-profile/vtune/` (L3 flush buffer + warmup + priming pattern).
- [ ] Add `external/` shellout wrappers for `perf record`, `cachegrind`, `heaptrack`; capture report paths in JSONL.
- [ ] Port `experiments/analyze_alloc_callstacks.py` to Rust under `sketch-profile/vtune/callstack.rs`.
- [ ] CI lint: forbid `sketch-profile` and `sketch-bench` from being deps of `sketch-runtime`.

### Phase 6 — Populate `sketch-runtime`

Embedded-benchmarking only (DESIGN §7.1). No `sketch-profile` deps (CI-enforced in Phase 5).

- [ ] Re-export `sketch-bench` metric types so the embedded path produces the **same `RunMetrics` record shape** as offline runs (DESIGN §5.8). No duplicate implementations.
- [ ] `sampler/`: `Sampler::every_n(N)` (every-Nth-op) and `Sampler::time_window(d)`; both implement `MetricsSink` (one `RunMetrics` per window).
- [ ] `exporter/`: `stdout`, `file`, `prometheus`, `grpc`.
- [ ] `proto/feedback.proto` — streaming schema for the ASAPController channel (version-pinned with `sketch-core` schema v1).
- [ ] Overhead bench `benches/sampler_overhead.rs` — must show ≤1% throughput loss at 1/1024 sampling vs a `Probe<_, NoopSink>` baseline.

### Phase 7 — Unified CLI `sketch-cli` / `sketchlib`

- [ ] `sketchlib bench …` — dispatches to `sketch-bench::BenchRunner`.
- [ ] `sketchlib profile …` — dispatches to `sketch-profile`.
- [ ] `sketchlib workload generate|describe` — the workload toolbox.
- [ ] CLI-side YAML config loader (sweep matrix with `{SIZE}` expansion, borrowed pattern from asap-fusion `experiments/configs/`); pairs with the `sketch-core/workload` side added in Phase 3 (DESIGN §8).
- [ ] Migrate `run_all_benchmarks.sh` to call `sketchlib` subcommands; keep the shell script as a thin wrapper until all binaries are gone.

### Phase 8 — Migrate legacy binaries + viz

- [ ] Replace each `rust/src/bin/*.rs` with a thin shim that calls into `sketch-bench::runner` (or delete once `sketchlib bench --sketch X --impl Y` covers it).
- [ ] Update `cpp/` harnesses to emit v1 JSONL directly (schema header shared via `sketch-core/schema/v1.json`).
- [ ] Update `visualization/` to consume the v1 schema; add a feature flag to still read legacy shapes for one release cycle, then remove.

### Phase 9 — Wire downstream apps

One at a time, in this order (cheapest / lowest blast radius first):

- [ ] DataCollector — add `sketch-core` + `sketch-bench` + `sketch-runtime` deps, wrap its sketches in `Probe`.
- [ ] ASAPQuery — same.
- [ ] asap-fusion — same; also retire asap-fusion's own `microbench/` scaffolding once the equivalents land in `sketch-profile`.
- [ ] ASAPController — consume the gRPC `feedback.proto` stream, match records against offline baselines.

### Phase 10 — Cleanup

- [ ] Delete `other_benchmark/`, `insertion_optimized_sample_output/`, loose `cumulative_result.txt` — once their data is re-captured under v1.
- [x] ~~Decide on repo rename: keep `sketchlib-bench` (history-friendly) or rename to `sketchlib-tool` (brand-consistent). Default: keep.~~ Done 2026-05-05: renamed to `sketch-bench`.
- [ ] Tag `v1.0` once phases 3–8 land.

---

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Big-bang refactor breaks ongoing benchmark work | Phases 1–4 are additive; legacy bins keep working until Phase 8. |
| Jemalloc dep forced onto downstream apps | `heap-jemalloc` is an opt-in feature; default memory metric is RSS. |
| `perf_event_open` requires elevated perms | Document `sysctl kernel.perf_event_paranoid` or capability setup in `sketch-profile/README.md`; profile runs fail gracefully. |
| Runtime exporter adds prod overhead | ≤1% budget enforced by CI microbench; disabled exporter is a no-op. |
| Schema churn breaks the visualization layer | `schema_version` field + one release of dual-read compatibility. |
| Downstream app coordination slips | Phase 9 doesn't block 1–8; apps can migrate on their own timeline as long as they pin versions. |
| History loss during subtree | `git subtree add --squash=false` preserves full history; verify with `git log --follow` on `sketch-core/src/workload/*`. |

---

## Done criteria

- [ ] `sketch-profiler` repo archived; all its non-stub code lives here with history.
- [ ] `sketchlib bench` and `sketchlib profile` cover every measurement currently produced by `run_all_benchmarks.sh`, at parity or better.
- [ ] Visualization reads the v1 schema exclusively.
- [ ] At least one downstream app (DataCollector) runs with `Probe<Sketch>` + `sketch-runtime` in production-like conditions, reporting to a test ASAPController.
- [ ] `sketch-runtime` sampler overhead bench ≤1%.
- [ ] `docs/DESIGN.md` matches shipped reality (no drift). -->
