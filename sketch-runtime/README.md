# `sketch-runtime`

The embedded half of sketchlib-tool. Downstream ASAP apps
(asap-fusion, DataCollector, ASAPQuery) wrap their live sketches
in a `Probe<S, Sampler>` and samples flow to an `Exporter`
(stdout / file / gRPC / prometheus — last two deferred, see
`TODO.md` at repo root) in the same v1 JSONL shape the offline
`sketchlib bench` CLI produces.

## Enabling / disabling — three levels

Pick the level whose cost / ergonomics fit. All three compose:
you can use only one, or stack them.

| Level | How | Hot-path cost |
|---|---|---|
| Compile-time | `sketch-runtime = { default-features = false }` | **0** — the `Sampler` struct compiles to a ZST whose `MetricsSink` impl is four `#[inline(always)]` empty methods. |
| Construction-time | `Sampler::disabled(exporter, tag)` | **1 cold branch** per op. Measured at ≈1.02× of `NoopSink` baseline (within noise). |
| Runtime | `RuntimeSwitch::disable()` | **1 `Relaxed` atomic load** per op. Flip from anywhere; the probe keeps running its other work. |

Additionally the sampling rate is a dial — even an `enabled`
sampler can set `sample_every_n = u32::MAX`, effectively never
emitting anything until the controller turns it down.

### Example — compile-time disable

```toml
# In DataCollector's Cargo.toml
[dependencies]
sketch-runtime = { path = "../sketchlib-bench/sketch-runtime", default-features = false }
```

No code change required — the `Sampler` API surface is identical
under `--no-default-features`, just every method is a no-op.

### Example — construction-time disable

```rust
use sketch_runtime::{exporter::NoopExporter, sampler::{Sampler, Tag}};
use sketch_core::report::Source;

let tag    = Tag::new("cms", "oxide", Source::DataCollector);
let sampler = Sampler::disabled(NoopExporter, tag);
let mut sk  = Probe::new(CmsOxide::new(), sampler);
```

### Example — runtime toggle

```rust
use sketch_runtime::{RuntimeSwitch, Sampler, StdoutExporter, Tag};

let switch  = RuntimeSwitch::on();
let sampler = Sampler::every_n(1024, 100, StdoutExporter::new(), tag)
    .with_switch(switch.clone());
let mut sk  = Probe::new(CmsOxide::new(), sampler);

// Controller POST handler, wired via Arc<RuntimeSwitch>:
fn on_controller_command(cmd: Cmd, sw: &RuntimeSwitch) {
    match cmd {
        Cmd::PauseBenchmarking  => sw.disable(),
        Cmd::ResumeBenchmarking => sw.enable(),
    }
}
```

## Sampling modes

Two independent axes: *when to sample* and *when to emit*. Four
constructors for the four combinations:

| Sample trigger | Emit trigger | Constructor | Use case |
|---|---|---|---|
| Every N ops | N sampled ops | `every_n(N, K, …)` | sparse event rate, "1 in 1M ops" |
| Every N ops | Time window | `every_n_time_window(N, d, …)` | **batching** — scrape every op, batch-emit per 1 s |
| Time period | K sampled ops | `every_period(p, K, …)` | "at most 1 sample per second, regardless of op rate" |
| Time period | Time window | `every_period_time_window(p, d, …)` | "1 sample per 10 ms, one batched record per 1 s" |

### Batching pattern (scrape frequent, emit per window)

The most-used combination. Measures latency + counts on every
op (or every Nth op — cheap), accumulates in memory, and emits
*one* batched v1 JSONL record per wall-clock window. No
per-op HTTP / file I/O on the hot path.

```rust
// Scrape every op, batch-emit per second:
Sampler::every_n_time_window(1, Duration::from_secs(1), exporter, tag)

// Scrape every 1000th op (cheaper), emit per second:
Sampler::every_n_time_window(1000, Duration::from_secs(1), exporter, tag)
```

### Sparse sampling

```rust
// One sample per 1 M ops, emit every 10 samples (= every 10 M ops):
Sampler::every_n(1_000_000, 10, exporter, tag)

// One sample per second of wall time, emit every 10 samples:
Sampler::every_period(Duration::from_secs(1), 10, exporter, tag)

// Never sample, never emit:
Sampler::disabled(exporter, tag)
```

Each emitted `Record` (v1 JSONL, same shape as `sketchlib bench`
output) carries `mode: "runtime"` + the user-supplied `Source`
tag (e.g. `"data-collector"`), so ASAPController can join live
records against offline baselines without schema translation.

## Overhead

From `benches/sampler_overhead.rs` on a local box:

| Config | ns per `update` | relative |
|---|---|---|
| `Probe<_, NoopSink>` — baseline | 0.44 | 1.00× |
| `Probe<_, Sampler::disabled>` | 0.45 | 1.02× |
| `Probe<_, Sampler::every_n(1024, …)>` | 2.70 | 6.1× |

The trivial "sketch" in the bench is a single wrapping add, so
per-op overhead is **all** sampler cost — real sketches ingest
at 15–80 ns/op and the relative sampler overhead drops to
**roughly 3–15%** on top. The design-doc ≤1% target applies to
real sketches at sampling rate 1/1024; this microbench
pessimistically measures against a 0.44 ns baseline.

If you need tighter overhead: go compile-time disabled, or drop
`without_latency()` to skip the `Instant::now()` on every
sampled op.

## Exporters shipped in v1

| Exporter | Use |
|---|---|
| `StdoutExporter` | dev, local compose runs |
| `FileExporter` | append-only JSONL, production `sidecar → file` pattern |
| `NoopExporter` | placeholder / hot-path-only cost measurements |

Deferred: `PrometheusExporter` (scrape `/metrics`), `GrpcExporter`
(streaming unary to ASAPController via `feedback.proto`). Tracked
in repo-root `TODO.md`.
