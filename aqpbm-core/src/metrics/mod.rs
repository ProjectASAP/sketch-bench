//! Metric recorders + the `FullSink` that composes them, plus the
//! `MetricsMask` that selects which recorders run.
//!
//! These are the generic measurement primitives — the wall / CPU
//! clocks, RSS + jemalloc + per-sketch heap tracking, the
//! throughput math, the per-run `RunMetrics` record, and the
//! `FullSink` that fills one. They carry no sketch-domain
//! knowledge, so they live in `aqpbm-core` where `sketch-bench`,
//! `sketch-runtime`, and any future `*-bench` share them without
//! depending on the sketch benchmark library. `MetricsMask` in
//! particular is named by both the offline `FullSink` and the
//! embedded `sketch-runtime::Sampler`.
//!
//! See `docs/DESIGN.md` §5.3 / §5.5.

#[cfg(feature = "heap-track")]
pub mod heap_track;
pub mod mask;
pub mod memory;
pub mod run;
pub mod throughput;
pub mod time;

// Re-exported here so `LatencyRecorder` / `LatencySnapshot` stay
// reachable alongside the other recorders even though the latency
// histogram itself lives in `crate::latency` (the embedded sampler
// wants it without the rest of the metric machinery).
pub use crate::latency::{LatencyRecorder, LatencySnapshot};
pub use mask::MetricsMask;
pub use memory::{JemallocAllocated, Rss};
pub use run::{FullSink, QueryCallSample, RunMetrics};
pub use throughput::ItemsPerSec;
pub use time::{CpuTimeSample, CpuTimeSampler, WallClock};
