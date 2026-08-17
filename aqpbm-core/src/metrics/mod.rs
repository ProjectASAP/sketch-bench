//! The metric recorders, plus the masks that select which of them a measurement
//! arms: wall / CPU clocks, RSS + jemalloc + heap tracking, the throughput math,
//! and the `RunMetrics` they fill. See `docs/aqpbm-core.md` §Metrics.

#[cfg(feature = "heap-track")]
pub mod heap_track;
pub mod mask;
pub mod memory;
pub mod run;
pub mod throughput;
pub mod time;

// Re-exported so `LatencyRecorder` / `LatencySnapshot` stay reachable beside the
// other recorders, though the histogram itself lives in `crate::latency`.
pub use crate::latency::{LatencyRecorder, LatencySnapshot};
pub use mask::{is_measurable, Metric, MetricsMask, Operation, OperationMask};
pub use memory::{JemallocAllocated, Rss};
pub use run::RunMetrics;
pub use throughput::ItemsPerSec;
pub use time::{CpuTimeSample, CpuTimeSampler, WallClock};
