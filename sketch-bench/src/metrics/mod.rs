//! Metric recorders + the `FullSink` that composes them.
//!
//! See `docs/DESIGN.md` §5.5.

pub mod latency;
pub mod memory;
pub mod throughput;
pub mod time;

use sketch_core::probe::MetricsSink;

use crate::config::MetricsMask;

pub use latency::LatencyRecorder;
pub use memory::{JemallocPeak, Rss};
pub use throughput::ItemsPerSec;
pub use time::{CpuTimeSample, CpuTimeSampler, WallClock};

/// Metrics produced by a single run. One `FullSink` finalises
/// into one of these. The `BenchRunner` aggregates `RunMetrics`
/// across N runs via the Welford accumulator.
#[derive(Debug, Clone, Default)]
pub struct RunMetrics {
    pub items_inserted: u64,
    pub queries_executed: u64,
    pub wall_time_ns: u64,
    pub insert_wall_time_ns: u64,
    pub query_wall_time_ns: u64,
    pub cpu_user_ns: Option<u64>,
    pub cpu_sys_ns: Option<u64>,
    pub rss_peak_kb: Option<u64>,
    /// Peak jemalloc-allocated bytes, in kB. Populated only when
    /// the `heap-jemalloc` feature is compiled into the binary
    /// AND the linking process has jemalloc as its global
    /// allocator. `None` otherwise.
    pub heap_peak_kb: Option<u64>,
    pub memory_bytes: Option<u64>,
    /// Latency histogram samples (ns/op) for the insert phase.
    /// None when `MetricsMask::LATENCY` unset.
    pub latency_ns: Option<LatencySnapshot>,
    /// Accuracy comparator output, carried opaquely as JSON so
    /// different ground-truth families can emit their own shape
    /// without leaking into the common metric struct.
    pub accuracy: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct LatencySnapshot {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
    pub count: u64,
}

/// Concrete sink for offline benchmark runs. Holds each
/// recorder only when the corresponding mask bit is set — so
/// `MetricsMask::THROUGHPUT` alone has no heap allocation for
/// the histogram, etc.
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

    /// Arm the run. Called by the runner once per measured
    /// iteration before any `update`/`query`.
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

    pub fn begin_query_phase(&mut self) {
        self.phase = Phase::Query;
        self.query_wall = Some(WallClock::start());
    }

    pub fn end_query_phase(&mut self) {
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
        let (rss_peak_kb, heap_peak_kb) = if self.mask.contains(MetricsMask::MEMORY) {
            (Rss::peak_kb(), JemallocPeak::peak_kb())
        } else {
            (None, None)
        };
        let latency_ns = self.latency.take().map(|rec| rec.snapshot());

        RunMetrics {
            items_inserted: self.items_inserted,
            queries_executed: self.queries_executed,
            wall_time_ns,
            insert_wall_time_ns,
            query_wall_time_ns,
            cpu_user_ns,
            cpu_sys_ns,
            rss_peak_kb,
            heap_peak_kb,
            memory_bytes,
            latency_ns,
            accuracy: None,
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
