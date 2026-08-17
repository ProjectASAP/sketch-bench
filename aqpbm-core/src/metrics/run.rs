//! The per-run metric record: one of these per measured run, filled by the
//! recorders `measure` arms and folded by `crate::run_stats`.

use std::collections::BTreeMap;

use crate::latency::LatencySnapshot;

/// Metrics produced by a single run. `crate::measure` fills one per run;
/// `crate::run_stats` folds them across N runs via the Welford accumulator.
#[derive(Debug, Clone, Default)]
pub struct RunMetrics {
    /// Units of work the timed region covered — items inserted, probes asked,
    /// shards folded. One measurement times one operation, so one count.
    pub work: u64,
    /// Wall time of the region the body marked. Setup is not in it.
    pub elapsed_ns: u64,
    pub cpu_user_ns: Option<u64>,
    pub cpu_sys_ns: Option<u64>,
    pub rss_peak_kb: Option<u64>,
    /// Currently-allocated jemalloc bytes (`stats.allocated`), in kB. Needs
    /// `heap-jemalloc` compiled in AND jemalloc as the process's global
    /// allocator. One sample, not a peak — see `heap_bytes_peak`.
    pub heap_allocated_kb: Option<u64>,
    /// The nominal footprint the body reported, read before it dropped
    /// anything it owned.
    pub memory_bytes: Option<u64>,
    /// Net bytes allocated over the body's lifetime, per the `heap-track`
    /// allocator. A body that owns and drops its sketch nets ~0 — the peak
    /// below is the one that means something there.
    pub heap_bytes_net: Option<u64>,
    /// High-water mark of the tracker across the body. Captures transient
    /// peaks (resize, intermediate buffers) that the net figure smooths over.
    pub heap_bytes_peak: Option<u64>,
    /// Per-call latency distribution, when the body used `Timed::time_each`
    /// and `MetricsMask::LATENCY` was set.
    pub latency_ns: Option<LatencySnapshot>,
    /// Named scalars the body reported — error metrics, probe counts. Flat
    /// rather than an opaque blob so `aggregate` folds every key across runs
    /// without knowing any algorithm's shape.
    pub scores: Option<BTreeMap<String, f64>>,
}
