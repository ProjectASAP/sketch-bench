//! The per-run metric record + the `FullSink` that composes the
//! recorders to fill one.
//!
//! See `docs/DESIGN.md` §5.5.

use std::collections::BTreeMap;

use crate::latency::{LatencyRecorder, LatencySnapshot};
use crate::metrics::mask::MetricsMask;
use crate::metrics::memory::{JemallocAllocated, Rss};
use crate::metrics::time::{CpuTimeSampler, WallClock};
use crate::probe::MetricsSink;

/// One row of per-call query telemetry: a 1-based call index, the timed estimate
/// call's wall, the answer, and — for quantile algorithms — the percentile and
/// repeat. Filled by the comparators in `accuracy`, which it knows nothing of.
#[derive(Debug, Clone, Copy)]
pub struct QueryCallSample {
    pub call_index: usize,
    pub nanoseconds: u64,
    pub estimate: f64,
    /// Percentile being queried (0..=100 fraction). NaN for
    /// cardinality / frequency algorithms.
    pub percentile: f64,
    /// Outer "repeat" index — each run sweeps the percentile array
    /// `REPEATS_PER_RUN` times to thicken the sample. 0 when it does not.
    pub repeat: usize,
}

/// Metrics produced by a single run. One `FullSink` finalises
/// into one of these. The `BenchRunner` aggregates `RunMetrics`
/// across N runs via the Welford accumulator.
#[derive(Debug, Clone, Default)]
pub struct RunMetrics {
    pub items_inserted: u64,
    pub queries_executed: u64,
    pub wall_time_ns: u64,
    /// Wall time of the insert loop alone — the **ingest** denominator.
    pub insert_wall_time_ns: u64,
    /// Wall time for `Accumulator::prepare()`, billed separately so deferred
    /// build cost shows without inflating insert or query; zero for a no-op.
    /// `insert` is the ingest rate, `insert + finalize` the queryable rate.
    pub finalize_wall_time_ns: u64,
    pub query_wall_time_ns: u64,
    pub cpu_user_ns: Option<u64>,
    pub cpu_sys_ns: Option<u64>,
    pub rss_peak_kb: Option<u64>,
    /// Currently-allocated jemalloc bytes (`stats.allocated`), in kB. Needs
    /// `heap-jemalloc` compiled in AND jemalloc as the process's global
    /// allocator. One sample at finalize, not a peak — see `heap_bytes_peak`.
    pub heap_allocated_kb: Option<u64>,
    pub memory_bytes: Option<u64>,
    /// Net bytes allocated to this sketch over its lifetime, per the
    /// `heap-track` allocator — steady state at the end of insert, complementing
    /// the param-derived `memory_bytes`. Needs `heap-track` + the bin's global.
    pub heap_bytes_net: Option<u64>,
    /// High-water mark of the tracker during construction + feed.
    /// Captures transient peaks (resize, intermediate buffers)
    /// that `heap_bytes_net` smooths over.
    pub heap_bytes_peak: Option<u64>,
    /// Latency histogram samples (ns/op) for the insert phase.
    /// None when `MetricsMask::LATENCY` unset.
    pub latency_ns: Option<LatencySnapshot>,
    /// Named accuracy scalars from this run's comparator. Flat rather
    /// than an opaque JSON blob so `aggregate` can fold every key across
    /// runs without knowing any algorithm's shape — see `accuracy::Comparison`.
    pub accuracy: Option<BTreeMap<String, f64>>,
    /// Per-call query samples — `Some` only when the frontend requested
    /// `record_calls` on a comparator that supports it. Rendered by
    /// `sketch-bench::legacy_csv`; not surfaced in the JSONL record.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

impl RunMetrics {
    /// All-zero metrics, for a pass that measures something other than the
    /// insert phase (the merge pass times a fold, not a loop over items).
    pub fn empty() -> Self {
        Self {
            items_inserted: 0,
            queries_executed: 0,
            wall_time_ns: 0,
            insert_wall_time_ns: 0,
            finalize_wall_time_ns: 0,
            query_wall_time_ns: 0,
            cpu_user_ns: None,
            cpu_sys_ns: None,
            rss_peak_kb: None,
            heap_allocated_kb: None,
            memory_bytes: None,
            heap_bytes_net: None,
            heap_bytes_peak: None,
            latency_ns: None,
            accuracy: None,
            query_calls: None,
        }
    }

    /// Wall time to turn this run's items into a *queryable* sketch — the
    /// denominator of `build_throughput_items_per_sec`. Saturating, so a bad sum
    /// across the two clocks pins the rate near zero instead of wrapping.
    pub fn build_wall_time_ns(&self) -> u64 {
        self.insert_wall_time_ns
            .saturating_add(self.finalize_wall_time_ns)
    }
}

/// Concrete sink for offline benchmark runs. Holds each recorder only when its
/// mask bit is set, so `MetricsMask::THROUGHPUT` alone allocates no histogram.
pub struct FullSink {
    mask: MetricsMask,
    wall: Option<WallClock>,
    insert_wall: Option<WallClock>,
    query_wall: Option<WallClock>,
    cpu: Option<CpuTimeSampler>,
    latency: Option<LatencyRecorder>,
    items_inserted: u64,
    queries_executed: u64,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Insert,
    Query,
}

impl FullSink {
    pub fn new(mask: MetricsMask) -> Self {
        Self {
            mask,
            wall: None,
            insert_wall: None,
            query_wall: None,
            cpu: None,
            latency: if mask.contains(MetricsMask::LATENCY) {
                Some(LatencyRecorder::new())
            } else {
                None
            },
            items_inserted: 0,
            queries_executed: 0,
            phase: Phase::Idle,
        }
    }

    /// Arm the run. Called by the runner once per measured iteration, before
    /// any `update`.
    pub fn on_run_start(&mut self) {
        self.wall = Some(WallClock::start());
        if self.mask.contains(MetricsMask::CPU) {
            self.cpu = Some(CpuTimeSampler::start());
        }
        self.phase = Phase::Idle;
        self.items_inserted = 0;
        self.queries_executed = 0;
    }

    pub fn begin_insert_phase(&mut self) {
        self.phase = Phase::Insert;
        self.insert_wall = Some(WallClock::start());
    }

    pub fn end_insert_phase(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Finalise the sink, consuming it to produce a
    /// `RunMetrics`.
    pub fn finalize(mut self, memory_bytes: Option<u64>) -> RunMetrics {
        let wall_time_ns = self.wall.take().map(|w| w.elapsed_ns()).unwrap_or(0);
        let insert_wall_time_ns = self.insert_wall.take().map(|w| w.elapsed_ns()).unwrap_or(0);
        let query_wall_time_ns = self.query_wall.take().map(|w| w.elapsed_ns()).unwrap_or(0);
        let (cpu_user_ns, cpu_sys_ns) = match self.cpu.take() {
            Some(sampler) => {
                let s = sampler.finish();
                (Some(s.user_ns), Some(s.sys_ns))
            }
            None => (None, None),
        };
        let (rss_peak_kb, heap_allocated_kb) = if self.mask.contains(MetricsMask::MEMORY) {
            (Rss::peak_kb(), JemallocAllocated::read_kb())
        } else {
            (None, None)
        };
        let latency_ns = self.latency.take().map(|rec| rec.snapshot());

        RunMetrics {
            items_inserted: self.items_inserted,
            queries_executed: self.queries_executed,
            wall_time_ns,
            insert_wall_time_ns,
            // `FullSink` never sees the finalize call — it lands outside any
            // hook this sink owns. `run_once` times it and overwrites this
            // field, keeping the sink's job to the per-update boundary.
            finalize_wall_time_ns: 0,
            query_wall_time_ns,
            cpu_user_ns,
            cpu_sys_ns,
            rss_peak_kb,
            heap_allocated_kb,
            memory_bytes,
            heap_bytes_net: None,
            heap_bytes_peak: None,
            latency_ns,
            accuracy: None,
            query_calls: None,
        }
    }
}

impl MetricsSink for FullSink {
    fn on_update_start(&mut self) {
        if let Some(rec) = &mut self.latency {
            rec.on_start();
        }
    }
    fn on_update_end(&mut self) {
        if self.phase == Phase::Insert {
            self.items_inserted = self.items_inserted.saturating_add(1);
        }
        if let Some(rec) = &mut self.latency {
            rec.on_end();
        }
    }
    fn on_query_start(&mut self) {}
    fn on_query_end(&mut self) {
        if self.phase == Phase::Query {
            self.queries_executed = self.queries_executed.saturating_add(1);
        }
    }
}
