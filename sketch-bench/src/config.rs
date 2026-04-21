//! `BenchConfig` + `MetricsMask` — knobs the caller sets before
//! running a benchmark.
//!
//! See `docs/DESIGN.md` §5.3.

use bitflags::bitflags;

bitflags! {
    /// Which metric families are collected during a run. Each
    /// bit gates both construction cost and hot-path overhead of
    /// its recorder — `Probe<_, FullSink>` only installs the
    /// recorders whose bits are set, and `FullSink` holds only
    /// those recorders. A mask with no bits set is legal (and
    /// useful as a minimal smoke-test).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MetricsMask: u32 {
        const THROUGHPUT = 1 << 0;
        const LATENCY    = 1 << 1;
        const CPU        = 1 << 2;
        const MEMORY     = 1 << 3;
        const ACCURACY   = 1 << 4;
    }
}

/// Configuration for one `BenchRunner` invocation.
#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Number of *measured* iterations. Aggregate stats use this
    /// population. Excludes warm-up.
    pub runs: usize,
    /// Number of warm-up iterations run before measurement
    /// begins. Lets jemalloc / CPU caches settle so the first
    /// measured run isn't artificially slow.
    pub warmup_runs: usize,
    /// Which metric families to record.
    pub metrics: MetricsMask,
    /// How many queries to run *per* measured iteration after
    /// the insert phase. `None` skips the query phase entirely
    /// (appropriate for CMS/CS-style sketches whose "query" is
    /// always paired with `update` in the downstream app).
    pub query_count: Option<usize>,
    /// Threads used for the insert phase. `1` is the default;
    /// multi-threaded support is planned but not wired.
    pub threads: usize,
    /// Seeds the sink's randomness (latency sampling boundary,
    /// future sampled ground-truth comparators, ...). Does NOT
    /// seed the workload — workloads own their own seed.
    pub seed: u64,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            runs: 10,
            warmup_runs: 3,
            metrics: MetricsMask::all(),
            query_count: None,
            threads: 1,
            seed: 0,
        }
    }
}
