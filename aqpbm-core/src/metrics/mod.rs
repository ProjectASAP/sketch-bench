//! Metric recorders + the `FullSink` that composes them, plus the `MetricsMask`
//! that selects which run: wall / CPU clocks, RSS + jemalloc + per-sketch heap
//! tracking, the throughput math, and the per-run `RunMetrics` record. None of
//! it carries sketch-domain knowledge. See `docs/DESIGN.md` §5.3 / §5.5.

#[cfg(feature = "heap-track")]
pub mod heap_track;
pub mod mask;
pub mod memory;
pub mod run;
pub mod throughput;
pub mod time;

// Re-exported so `LatencyRecorder` / `LatencySnapshot` stay reachable beside the
// other recorders, though the histogram lives in `crate::latency` — the embedded
// sampler wants it without the rest of the metric machinery.
pub use crate::latency::{LatencyRecorder, LatencySnapshot};
pub use mask::MetricsMask;
pub use memory::{JemallocAllocated, Rss};
pub use run::{FullSink, QueryCallSample, RunMetrics};
pub use throughput::ItemsPerSec;
pub use time::{CpuTimeSample, CpuTimeSampler, WallClock};
