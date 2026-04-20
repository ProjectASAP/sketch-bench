# `sketchlib-tool` — Design

Evolution of `sketchlib-bench` into a unified **benchmarking + profiling** tool and **embeddable library** for sketch algorithms used across the ASAP project (asap-fusion, DataCollector, ASAPQuery, ASAPController).

> Repo name remains `sketchlib-bench`. The shipped tool/brand is `sketchlib-tool`; the CLI binary is `sketchlib`.

---

## 1. Goals

Produce **one crate graph** that serves three audiences:

| Audience | Mode | What they want |
|---|---|---|
| Developers / researchers | Offline CLI | Reproducible benchmark + profile runs against sketch implementations |
| Downstream ASAP apps (asap-fusion, DataCollector, ASAPQuery) | Embedded library, runtime | Always-on, low-overhead metrics from live sketch usage |
| ASAPController control plane | Consumer | Structured metric stream from running apps for online decisions |

### 1.1 Non-goals

- Not a replacement for `perf` / VTune — we *use* them, we don't reimplement.
- Not a general-purpose observability library — scoped to sketch workloads.
- No backwards-compatibility shims for the legacy bin names; they get migrated in-place.

---

## 2. Terminology (precise, and enforced in code)

We keep these two concepts strictly separate throughout the code, docs, CLI, and report schema.

### 2.1 Benchmark = MACRO metrics

Measures end-to-end behavior of a sketch on a workload. Low overhead. Appropriate for both offline runs and always-on embedding.

- Throughput (items/sec) for `insert` and `query`
- Latency distribution (p50/p95/p99) via `hdrhistogram`
- Wall-clock time + CPU time (user + sys) via `getrusage`
- Peak resident set (RSS) from `/proc/self/status:VmHWM`
- Peak heap via `tikv-jemalloc-ctl`
- Accuracy: per-sketch ground-truth comparison (L1/L2 error, relative error, rank error, etc.)
- Confidence: mean, stddev, and 95% CI across N runs

### 2.2 Profile = MICRO metrics (VTune-style)

Measures microarchitectural and allocation behavior. Higher overhead, usually CLI-only.

- Hardware counters via `perf_event_open`: L1/L2/LLC references and misses, branch instructions and mispredicts, dTLB/iTLB misses, instructions, cycles (→ IPC, miss rates)
- Sampling profiles via `perf record` → flamegraph
- Deterministic cache simulation via `cachegrind`
- Allocation profiles via `heaptrack`
- VTune callstack CSV ingestion (borrowed from asap-fusion `experiments/analyze_alloc_callstacks.py`)

Benchmark and profile are **peer subsystems**, not parent/child.

---

## 3. Crate layout

```
sketchlib-bench/            (repo)
├── sketch-core/            # shared: workload, sketch_probe trait, report schema, serde types
├── sketch-bench/           # MACRO metrics (§2.1)
│   ├── metrics/            # wall-time, cpu-time, rss, jemalloc, hdrhist
│   ├── accuracy/           # ground-truth comparison
│   ├── aggregation/        # N-run mean/stddev/95% CI
│   └── runner/
├── sketch-profile/         # MICRO metrics (§2.2)
│   ├── hw_counters/        # perf_event_open: cache/branch/TLB/IPC
│   ├── external/           # perf record, cachegrind, heaptrack shellout
│   └── vtune/              # borrowed vtune harness + callstack CSV analyzer
├── sketch-runtime/         # embedded layer for downstream apps
│   ├── sampler/            # every-Nth-op / time-window sampling
│   └── exporter/           # stdout, file, prometheus, grpc-to-controller
├── sketch-cli/             # unified binary `sketchlib`: `bench` / `profile` subcommands
├── cpp/  rust/             # existing Rust/C++ benches — migrated onto sketch-core/bench
├── accuracy/ throughput/   # existing — migrated onto sketch-bench
├── input/  scripts/        # shared datasets + generators
└── visualization/          # JSON/JSONL viewer (kept, fed by unified report schema)
```

Top-level `Cargo.toml` becomes a workspace over the `sketch-*` crates plus the existing `rust/` and `benchmark/` members.

### 3.1 Dependency direction

```
sketch-cli  →  sketch-bench, sketch-profile, sketch-runtime  →  sketch-core
                                                  ↓
                                   (downstream apps depend here)
```

Downstream apps pull `sketch-core + sketch-bench + sketch-runtime` only. They do **not** take `sketch-profile` — its `perf_event_open` / VTune / cachegrind deps are CLI-only.

---

## 4. Core abstractions (`sketch-core`)

### 4.1 `Sketch` trait

```rust
pub trait Sketch {
    type Input<'a>;
    type Query;
    type Estimate;

    fn update(&mut self, v: Self::Input<'_>);
    fn bulk_update(&mut self, vs: &[Self::Input<'_>]) {
        for v in vs { self.update(*v); }
    }
    fn query(&self, q: Self::Query) -> Self::Estimate;
    fn memory_bytes(&self) -> usize;
}
```

Implementations live beside each sketch in `rust/src/` (no forced relocation). Third-party crates (`asap_sketchlib`, `sketch_oxide`, `datasketches`) are wrapped in thin newtypes to fit the trait.

### 4.2 `Probe` decorator

Newtype wrapper that intercepts every `update` / `query` and feeds a `MetricsSink`. Used identically by offline `sketch-bench` and embedded `sketch-runtime`:

```rust
pub struct Probe<S: Sketch, Sink: MetricsSink> { inner: S, sink: Sink }

impl<S: Sketch, Sink: MetricsSink> Sketch for Probe<S, Sink> { /* timing hooks */ }
```

### 4.3 `Workload`

Absorbed from `sketch-profiler`'s `data_gen/`:

- `DataShape { data_size, cardinality_shape }`
- `CardinalityShape { total_cardinality, unnormalized_heavy_cardinality }` (heavy hitters + tail)
- YAML config loader (pattern borrowed from asap-fusion `experiments/configs/`, with `{SIZE}` placeholder expansion)
- Distribution generators: uniform, Zipf (s-parameter), custom shape

### 4.4 Report schema (JSONL, one record per run)

```json
{
  "schema_version": 1,
  "sketch": "cms",
  "impl": "sketch_oxide",
  "workload": {"shape": "zipf", "s": 1.1, "size": 1000000},
  "mode": "bench",              // or "profile"
  "runs": 10,
  "bench": {
    "throughput_items_per_sec": {"mean": 4.2e7, "stddev": 1.1e6, "ci95": [4.15e7, 4.25e7]},
    "latency_ns":   {"p50": 21, "p95": 48, "p99": 120},
    "cpu_time_ms":  {"user": 231, "sys": 12},
    "rss_peak_kb":  18340,
    "heap_peak_kb": 16211,
    "accuracy":     {"l1_err_mean": 0.021, "rank_err_p99": 0.04}
  },
  "profile": {
    "hw_counters": {"l1d_miss_rate": 0.018, "llc_miss_rate": 0.004, "ipc": 3.2, "branch_miss_rate": 0.003},
    "external":    {"cachegrind_report": "…/cachegrind.out.123", "flamegraph": "…/flame.svg"}
  },
  "source": "cli",              // or "asap-fusion" / "DataCollector" / "ASAPQuery"
  "timestamp": "2026-04-20T12:34:56Z"
}
```

Offline runs fill `bench` and/or `profile`. Runtime samples emit the same record with `source != "cli"` and typically only `bench.*` populated.

---

## 5. CLI (`sketch-cli` → `sketchlib`)

```
sketchlib bench    --sketch hll  --impl oxide --workload zipf-s1.1 --size 1M --runs 10 \
                   --metrics throughput,latency,accuracy,memory --report out.jsonl

sketchlib profile  --sketch hll  --impl oxide --workload zipf-s1.1 --size 1M \
                   --counters cache,branch,tlb  [--perf-record] [--cachegrind] [--heaptrack] \
                   --report out.jsonl

sketchlib workload generate  --shape zipf --s 1.1 --size 1M --out input/zipf_1m.bin
sketchlib workload describe  input/zipf_1m.bin
```

Both subcommands write the same JSONL schema, so `visualization/` consumes either.

---

## 6. Runtime / controller feedback loop

### 6.1 Embedded usage (asap-fusion, DataCollector, ASAPQuery)

```rust
use sketch_core::Probe;
use sketch_runtime::{Sampler, exporter::GrpcExporter};

let exporter = GrpcExporter::connect("asapcontroller:9090")?;
let sampler  = Sampler::every_n(1024);                          // 1 sample per 1024 ops
let mut sketch = Probe::new(CmsLib::new(width, depth), sampler.with_exporter(exporter));

sketch.update(item);            // measured on the sampled path; pass-through otherwise
```

Overhead target: `<1%` throughput loss at sampling rate 1/1024. Enforced by a microbench in `sketch-runtime/benches/`.

### 6.2 Exporters

- `stdout`, `file` — dev defaults
- `prometheus` — scrape endpoint (for generic observability stacks)
- `grpc` — streaming unary calls to ASAPController (proto in `sketch-runtime/proto/feedback.proto`)

### 6.3 Controller loop (sketch of interaction)

```
 asap-fusion / DataCollector / ASAPQuery
        │   Probe<Sketch>
        ▼
 sketch-runtime::Sampler  ──── grpc ────▶ ASAPController
        │                                        │
 (same JSONL schema as offline)                  │
                                                 ▼
                                 decisions: switch impl, resize,
                                 tune sampling, re-route workload
```

The controller can compare live samples against an offline baseline (same schema) to detect drift (e.g. live `llc_miss_rate` is 3× baseline → downgrade to a cache-friendlier impl).

---

## 7. Code we borrow from asap-fusion

All adapted, not vendored blindly.

| From asap-fusion | Into sketchlib-tool | Why |
|---|---|---|
| `microbench/sketchlib_rust/kll_optimizations/sketchlib_kll_vtune.rs` | `sketch-profile/vtune/` harness | L3-flush + warmup + ns/item reporting template |
| `microbench/sketchlib_rust/cms_optimizations/cms_cache_efficiency.rs` | `sketch-profile/hw_counters/` example | Item-major vs row-major pattern |
| `experiments/asap_bench.py` + YAML | `sketch-cli` config loader + `sketch-core/workload` | YAML-driven sweep matrix, `{SIZE}` expansion |
| `experiments/analyze_alloc_callstacks.py` | `sketch-profile/vtune/callstack.rs` (Rust port) | VTune callstack CSV → allocation hotspots |
| `microbench/sketchlib_rust/kll_optimizations/kll99_without_*.rs` | `sketch-profile/ablation/` | Ablation-study pattern |
| `docs/asap_sketchlib_feedback_v*.md` | `docs/FEEDBACK_LOG.md` (new convention) | Formalizes the bench → insight → library-change loop |

---

## 8. Report compatibility with existing outputs

Existing JSONL under `cpp/output/`, `rust/output/`, `accuracy/`, `throughput/` uses ad-hoc shapes (`{implementation_name, total_nanoseconds}` etc.). Migration:

1. `sketch-bench` adapters read the legacy shape and re-emit the v1 schema in `§4.4`.
2. Visualization layer `visualization/` is updated to consume the v1 schema; legacy viewer kept one release cycle.
3. Once all benches emit v1, legacy shapes are removed.

---

## 9. Overhead + correctness invariants

- `sketch-runtime` sampler must cost ≤1% throughput at sample rate 1/1024 — enforced by criterion bench `sketch-runtime/benches/sampler_overhead.rs`.
- `Probe<S>` without a sink configured is a zero-cost wrapper (`#[inline]`, `PhantomData`).
- `sketch-profile` never imported by `sketch-runtime` (checked by a dependency-direction CI lint).
- One JSONL schema version across offline + runtime; bumping it is a coordinated change.

---

## 10. Open questions

- **Jemalloc vs default allocator for embedded consumers.** `tikv-jemalloc-ctl` gives precise heap peak but forces jemalloc on the linking binary. Possible resolution: feature flag `heap-jemalloc` (off by default), fall back to RSS-only.
- **Controller proto stability.** Who owns `feedback.proto` — here, or in ASAPController? Recommendation: here, versioned, imported by controller.
- **C++ benches in the new world.** C++ binaries can write v1 JSONL directly (simpler) or call through an FFI boundary to `sketch-core` (more uniform, higher cost). Recommend direct JSONL writes with a shared schema header.
- **Legacy datasets.** `input/benchmark_data_*.bin` stay; new workloads generated by `sketchlib workload generate` land alongside.
