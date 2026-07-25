//! `BenchConfig` — knobs the caller sets before running a
//! benchmark. `MetricsMask` is re-exported alongside it.
//!
//! See `docs/DESIGN.md` §5.3.

// `MetricsMask` lives in `crate::metrics` so that `sketch-runtime`
// can name it without depending on the runner. Re-exported here so
// callers reaching for the runner's config find it in one place.
pub use crate::metrics::MetricsMask;

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
    /// Number of shards the merge pass splits the stream into. `1` means the
    /// merge pass has nothing to fold and is skipped. Contiguous ranges, and
    /// folded sequentially into one accumulator — the partitioning scheme and
    /// the fold topology are both real experimental axes (a KLL's error
    /// depends on both), but v1 fixes them and names them here rather than
    /// pretending the choice does not exist.
    pub merge_shards: usize,
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
            merge_shards: 1,
            seed: 0,
        }
    }
}
