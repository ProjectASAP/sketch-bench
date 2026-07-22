# `sketchlib-tool` — Design

Evolution of `sketch-bench` into a unified **benchmarking + profiling** tool and **embeddable library** for sketch algorithms used across the ASAP project (asap-fusion, DataCollector, ASAPQuery, ASAPController).

> Repo name is `sketch-bench`. The shipped tool/brand is `sketchlib-tool`; the CLI binary is `sketchlib`.

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
sketch-bench/               (repo)
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
├── cpp/                    # existing C++ benches — migrated onto sketch-core/bench
├── accuracy/ throughput/   # existing — migrated onto sketch-bench
├── input/  scripts/        # shared datasets + generators
└── visualization/          # JSON/JSONL viewer (kept, fed by unified report schema)
```

Top-level `Cargo.toml` becomes a workspace over the `sketch-*` crates plus the existing `rust/` member. The Criterion micro-bench crate now lives in `rust-microbench/` (renamed from `benchmark/`) and is kept as a standalone cargo project, not a workspace member.

### 3.1 Dependency direction

```
sketch-cli  →  sketch-bench, sketch-profile  →  sketch-core
                                                     ↑
                             sketch-runtime  ────────┘
                                    ↑
                     (downstream apps depend here)
```

Downstream apps pull `sketch-core + sketch-runtime` only. They take neither `sketch-profile` — its `perf_event_open` / VTune / cachegrind deps are CLI-only — nor `sketch-bench`.

`sketch-runtime` depending on `sketch-bench` would defeat the split: the runtime is linked into always-on production binaries, and the offline benchmark library (runner, baselines, accuracy comparators, `tikv-jemalloc-ctl`) has no business in one. The two types both layers need — `MetricsMask` and `LatencyRecorder` — therefore live in `sketch-core`, not in `sketch-bench`.

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

**Implemented** as the `sketch_core::datagen` module + the `sketchlib
workload generate|describe` subcommand — see `docs/DATAGEN.md`. The
extensible `Shape` registry currently ships uniform, Zipf,
monotonic-timestamp (inter-arrival gaps), and skewed-categorical
(finite-domain) shapes over i64/u64/f64 output, written as a raw
little-endian `.bin` (consumed by `bench --input`) plus a `.meta.json`
provenance sidecar. YAML/JSON specs cover the `{SIZE}`/custom-shape
config intent.

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
    "throughput_items_per_sec": {"mean": 4.2e7, "stddev": 1.1e6, "n": 10},
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

## 5. `sketch-bench` library — detailed design

`sketch-bench` is the concrete benchmark library built on top of `sketch-core`. The same crate is invoked by `sketch-cli` for offline runs; downstream apps consume a thinner interface via `sketch-runtime` (see §7), but both share the types and metric definitions described here.

### 5.1 Design principles

- **Explicit state.** Measurement state lives in a `Sink`; no thread-locals.
- **Zero cost when off.** `MetricsMask` compiled down so `Probe` with a `NoopSink` is a pass-through.
- **Fresh state per run.** Multi-run statistics require independent initial conditions — the runner takes a factory closure, not a single sketch.
- **Same record shape offline and online.** `RunMetrics` is the unit of output; offline N-run mean/stddev is built from the same records that runtime samplers emit.
- **No panics on sampled paths.** Errors are captured in the report, never unwound into app code.

### 5.2 Module layout

```
sketch-bench/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── runner.rs            # BenchRunner: workload + factory → BenchReport
│   ├── config.rs            # BenchConfig, MetricsMask
│   ├── report.rs            # BenchReport → sketch-core v1 JSONL
│   ├── metrics/
│   │   ├── mod.rs           # MetricsSink trait; NoopSink, FullSink
│   │   ├── time.rs          # WallClock, CpuTime (getrusage)
│   │   ├── latency.rs       # LatencyRecorder (hdrhistogram)
│   │   ├── memory.rs        # Rss (/proc/self/status), JemallocPeak
│   │   └── throughput.rs    # ItemsPerSec
│   ├── accuracy/
│   │   ├── mod.rs           # GroundTruth + Comparator traits
│   │   ├── frequency.rs     # CMS/CS: L1, L2, relative error
│   │   ├── cardinality.rs   # HLL: relative error
│   │   ├── quantile.rs      # KLL: rank error grid
│   │   └── topk.rs          # precision@k, recall@k
│   └── aggregation/
│       ├── mod.rs           # RunStats<T>: mean/stddev/95% CI
│       └── welford.rs       # numerically-stable online accumulator
├── benches/                 # self-overhead + aggregator perf
└── tests/
```

### 5.3 Public API

#### `BenchConfig`

```rust
bitflags::bitflags! {
    pub struct MetricsMask: u32 {
        const THROUGHPUT = 1 << 0;
        const LATENCY    = 1 << 1;
        const CPU        = 1 << 2;
        const MEMORY     = 1 << 3;
        const ACCURACY   = 1 << 4;
    }
}

pub struct BenchConfig {
    pub runs: usize,                 // e.g. 10
    pub warmup_runs: usize,          // e.g. 3
    pub metrics: MetricsMask,
    pub query_count: Option<usize>,  // None = skip query phase
    pub threads: usize,              // 1 for single-threaded
    pub seed: u64,                   // reproducibility
}
```

#### `BenchRunner`

```rust
pub struct BenchRunner<S, W, G = ()>
where
    S: Sketch,
    W: Workload<Item = S::Input<'static>>,
    G: GroundTruth<S>,
{
    config: BenchConfig,
    workload: W,
    ground_truth: Option<G>,
}

impl<S, W, G> BenchRunner<S, W, G> {
    pub fn new(config: BenchConfig, workload: W) -> Self;
    pub fn with_ground_truth(self, g: G) -> Self;
    /// Factory is called once per run to get an independent fresh sketch.
    pub fn run<F: FnMut() -> S>(&self, factory: F) -> BenchReport;
}
```

The factory argument is deliberate: N-run CI requires independent initial state — sharing a sketch across runs would correlate the samples.

#### `BenchReport`

```rust
pub struct BenchReport {
    pub sketch: String,
    pub impl_name: String,
    pub workload_desc: WorkloadDesc,
    pub per_run: Vec<RunMetrics>,      // length == config.runs (warmup excluded)
    pub aggregated: AggregatedMetrics, // mean/stddev/CI over per_run
}

impl BenchReport {
    pub fn to_jsonl(&self) -> String;   // v1 schema (§4.4)
    pub fn write_to(&self, path: &Path) -> io::Result<()>;
}
```

#### `MetricsSink` (the lower-level hook)

```rust
pub trait MetricsSink {
    fn on_update_start(&mut self);
    fn on_update_end(&mut self);
    fn on_query_start(&mut self);
    fn on_query_end(&mut self);
    fn finalize(self) -> RunMetrics;
}
```

Built-in sinks: `NoopSink` (passthrough), `FullSink { mask: MetricsMask }` (offline), `SampledSink` in `sketch-runtime` (for embedded use).

`Probe<S, Sink>` from `sketch-core` (§4.2) is the only caller of these hooks — which is why the same sketch wrapper serves offline benchmarks and runtime samplers.

### 5.4 End-to-end flow

```
(BenchConfig, Workload, Factory<Sketch>, Optional<GroundTruth>)
          │
          ▼
 items = workload.generate()
          │
          ▼
 for run in 0..(warmup_runs + runs):
     sketch = factory()
     sink   = FullSink::new(config.metrics)
     probe  = Probe::new(sketch, sink)
     sink.on_run_start()
       for item in &items { probe.update(item); }
       if let Some(n) = config.query_count {
           for q in workload.queries(n) { probe.query(q); }
       }
     sink.on_run_end()
     if run >= warmup_runs:
         metrics  = sink.finalize()
         accuracy = ground_truth.as_ref()
                        .map(|g| g.compare(&probe, &items))
         per_run.push(metrics.with_accuracy(accuracy))
          │
          ▼
 aggregated = Welford::aggregate(&per_run)
          │
          ▼
 BenchReport → v1 JSONL (§4.4)
```

### 5.5 Metric collection mechanics

| Metric | Mechanism | Notes |
|---|---|---|
| Throughput | `items.len() as f64 / wall_elapsed.as_secs_f64()` | Split for insert and query phases separately |
| Latency | `hdrhistogram::Histogram<u64>` | Only armed when `MetricsMask::LATENCY` set — zero cost otherwise |
| Wall clock | `std::time::Instant` at run boundaries | |
| CPU time | `libc::getrusage(RUSAGE_SELF)` delta (`utime + stime`) | Process only, not children |
| RSS peak | parse `/proc/self/status:VmHWM` at run end | Linux-only; documented |
| Heap peak | `tikv_jemalloc_ctl::stats::allocated` sampled | Feature `heap-jemalloc`; `None` when off |
| Accuracy | sketch-family `GroundTruth::compare` | See §5.6 |

Each metric lives behind a bit in `MetricsMask`. A sink constructs only the recorders its mask enables, so an ACCURACY-only run pays no latency-histogram cost.

### 5.6 Accuracy: the `GroundTruth` trait

```rust
pub trait GroundTruth<S: Sketch> {
    type Comparison: Serialize;
    fn compare(&self, sketch: &S, items: &[S::Input<'_>]) -> Self::Comparison;
}
```

Built-in families:

- **Frequency** (CMS, CS) — exact per-key counts from `items`; report `{l1_err, l2_err, relative_err_mean, relative_err_p99}`.
- **Cardinality** (HLL) — distinct-value count from a `HashSet` over `items`; report `{relative_err_mean, relative_err_p99}`.
- **Quantile** (KLL) — exact sorted copy; report `max_rank_err` over a 101-point quantile grid.
- **Top-k** — exact top-k from a `HashMap` count; report `{precision_at_k, recall_at_k}`.

Downstream apps can implement their own `GroundTruth` for domain-specific comparisons.

### 5.7 Aggregation — Welford, and where a CI may come from

`aggregation/welford.rs` is a numerically-stable online accumulator; called once per metric across the N post-warmup runs:

```rust
pub struct RunStats {
    pub n: usize,
    pub mean: f64,
    pub stddev: f64,           // sample stddev (n-1 divisor)
    pub ci95: Option<[f64; 2]>, // present only across processes — see below
}
```

**`ci95` is absent unless `sketchlib bench --repeats R` (R > 1) produced it.**

This section used to prescribe the opposite: it computed the interval over the
N post-warmup runs of a single invocation and told the reader that a wide
interval was "easy to tighten by raising `BenchConfig::runs`". That advice was
backwards. Those N runs share one process — one allocator arena, one
address-space layout, one governor ramp, one already-resident item slice — so
they are not independent samples of the implementation's throughput. They
estimate how much the last few seconds of that process wobbled. Dividing their
spread by `sqrt(N)` produced an interval far tighter than the command's own
reproducibility, and raising N made it narrower and *more* wrong. Four
identical invocations produced four mutually disjoint 95% intervals; measured
on `cms/lib-fixedmatrix-fast-32k`, the cross-process stddev was **3x** the
within-process stddev.

So a repeat is a whole process (`sketch-cli/src/repeat.rs`): `--repeats R`
re-executes this binary R times over byte-identical argv and computes the
interval from the R per-process means, with `n = R`. Within one process,
`--runs N` still reports `mean` / `stddev` / `throughput_samples` — none of
which claim to be an inference about a population — and no interval.

Accuracy does not use this axis: a sketch's error is a deterministic function
of (data, parameters) with no process-level variance to sample. Its
repetitions vary the data instead.

### 5.8 Relationship to `sketch-runtime`

`sketch-runtime` does **not** duplicate the metric implementations, and does not depend on `sketch-bench` to reach them — the recorders both layers share (`MetricsMask`, `LatencyRecorder`) live in `sketch-core` (§4). On top of those it provides:

- `SampledSink`: a `MetricsSink` that only records on 1/N-th or time-windowed ops.
- `Exporter` trait + `stdout | file | prometheus | grpc` implementations.
- A per-window `finalize()` that emits the same `RunMetrics` shape, tagged with a `source` field (`"asap-fusion"`, `"DataCollector"`, …).

So the controller sees the same records whether they came from an offline CI run or a live app — which is what makes the feedback loop in §7 possible.

### 5.9 Worked example — benchmarking HLL

```rust
use sketch_bench::{BenchRunner, BenchConfig, MetricsMask};
use sketch_bench::accuracy::cardinality::CardinalityGT;
use sketch_core::workload::Zipf;

let cfg = BenchConfig {
    runs: 10,
    warmup_runs: 3,
    metrics: MetricsMask::all(),
    query_count: Some(1),          // HLL has a single cardinality query
    threads: 1,
    seed: 42,
};
let workload = Zipf::new(1.1, 1_000_000);
let gt       = CardinalityGT::from_workload(&workload);

let report = BenchRunner::new(cfg, workload)
    .with_ground_truth(gt)
    .run(|| hll_oxide::HyperLogLog::new(14));

report.write_to(std::path::Path::new("output/hll_oxide.jsonl"))?;
```

Emitted JSONL (abbreviated):

```json
{ "schema_version":1, "sketch":"hll", "impl":"sketch_oxide",
  "workload":{"shape":"zipf","s":1.1,"size":1000000},
  "mode":"bench", "runs":10,
  "bench":{
    "throughput_items_per_sec":{"mean":4.2e7,"stddev":1.1e6,"n":10},
    "latency_ns":{"p50":21,"p95":48,"p99":120},
    "cpu_time_ms":{"user":231,"sys":12},
    "rss_peak_kb":18340, "heap_peak_kb":null,
    "accuracy":{"relative_error_mean":0.008,"relative_error_p99":0.031}
  },
  "source":"cli","timestamp":"..."
}
```

### 5.10 Cargo features

| Feature | Default | Effect |
|---|---|---|
| `heap-jemalloc` | off | Enables `tikv-jemalloc-ctl` heap-peak metric; forces jemalloc on the linking binary |
| `hdrhist` | on | Enables `hdrhistogram` latency recorder (off → latency mask is a no-op) |
| `accuracy-topk` | on | Pulls `HashMap`-based top-k ground truth |

Downstream apps pick the minimum set they need to keep their binary small.

---

## 6. CLI (`sketch-cli` → `sketchlib`)

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

## 7. Runtime / controller feedback loop

### 7.1 Embedded benchmarking (asap-fusion, DataCollector, ASAPQuery)

**Scope: bench-mode metrics only.** The embedded path collects macro metrics (throughput, latency, CPU, memory, accuracy) — the same `RunMetrics` shape produced by `BenchRunner` (§5), just emitted per sampling window instead of per offline run. Profile-mode metrics (hw counters, cachegrind, heaptrack, perf record, VTune) are **not** available here: `sketch-profile` is CLI-only and forbidden as a dep of `sketch-runtime`, as is `sketch-bench` (§3.1, §10). Those tools are too expensive for always-on embedding.

```rust
use sketch_core::Probe;
use sketch_runtime::{Sampler, exporter::GrpcExporter};

let exporter = GrpcExporter::connect("asapcontroller:9090")?;
let sampler  = Sampler::every_n(1024);                          // 1 sample per 1024 ops
let mut sketch = Probe::new(CmsLib::new(width, depth), sampler.with_exporter(exporter));

sketch.update(item);            // measured on the sampled path; pass-through otherwise
```

Overhead target: `<1%` throughput loss at sampling rate 1/1024. Enforced by a microbench in `sketch-runtime/benches/`.

### 7.2 Exporters

- `stdout`, `file` — dev defaults
- `prometheus` — scrape endpoint (for generic observability stacks)
- `grpc` — streaming unary calls to ASAPController (proto in `sketch-runtime/proto/feedback.proto`)

### 7.3 Controller loop (sketch of interaction)

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

## 8. Code we borrow from asap-fusion

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

## 9. Report compatibility with existing outputs

Existing JSONL under `cpp/output/`, `accuracy/`, `throughput/` uses ad-hoc shapes (`{implementation_name, total_nanoseconds}` etc.). Migration:

1. `sketch-bench` adapters read the legacy shape and re-emit the v1 schema in `§4.4`.
2. Visualization layer `visualization/` is updated to consume the v1 schema; legacy viewer kept one release cycle.
3. Once all benches emit v1, legacy shapes are removed.

---

## 10. Overhead + correctness invariants

- `sketch-runtime` sampler must cost ≤1% throughput at sample rate 1/1024 — enforced by criterion bench `sketch-runtime/benches/sampler_overhead.rs`.
- `Probe<S>` without a sink configured is a zero-cost wrapper (`#[inline]`, `PhantomData`).
- `sketch-profile` never imported by `sketch-runtime` (checked by a dependency-direction CI lint).
- One JSONL schema version across offline + runtime; bumping it is a coordinated change.

---

## 11. Open questions

- **Jemalloc vs default allocator for embedded consumers.** `tikv-jemalloc-ctl` gives precise heap peak but forces jemalloc on the linking binary. Possible resolution: feature flag `heap-jemalloc` (off by default), fall back to RSS-only.
- **Controller proto stability.** Who owns `feedback.proto` — here, or in ASAPController? Recommendation: here, versioned, imported by controller.
- **C++ benches in the new world.** C++ binaries can write v1 JSONL directly (simpler) or call through an FFI boundary to `sketch-core` (more uniform, higher cost). Recommend direct JSONL writes with a shared schema header.
- **Legacy datasets.** `input/benchmark_data_*.bin` stay; new workloads generated by `sketchlib workload generate` land alongside.
