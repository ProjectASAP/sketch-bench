//! `BenchConfig` — knobs the caller sets before running a
//! benchmark. `MetricsMask` is re-exported alongside it.
//!
//! See `docs/DESIGN.md` §5.3.

// `MetricsMask` lives in `crate::metrics` so that `sketch-runtime`
// can name it without depending on the runner. Re-exported here so
// callers reaching for the runner's config find it in one place.
pub use crate::metrics::{MetricsMask, OperationMask};

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
    /// Which metric algorithms to record.
    pub metrics: MetricsMask,
    /// Which operations to record them over. The request is the cross product
    /// of this and `metrics`; a cell with no implementation simply produces
    /// nothing.
    pub operations: OperationMask,
    /// Reserved: how many queries to run per measured iteration. The runner
    /// does not read it — the `GroundTruth` comparators own the query phase
    /// and pick their own probe counts.
    pub query_count: Option<usize>,
    /// Worker threads for the insert phase, read by the parallel-insert cells.
    /// `1` — a single-threaded run — for every other row.
    pub threads: usize,
    /// How many shards the merge operation folds. A knob, never a selector:
    /// whether merge is measured is decided by `operations`. Contiguous ranges
    /// folded sequentially into one accumulator; both the partitioning and the
    /// fold topology are real axes, fixed here for now.
    pub merge_shards: usize,
    /// The run's nominal seed, carried through to the legacy CSV's `seed`
    /// column. Does NOT seed the workload — workloads own their own seed.
    pub seed: u64,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            runs: 10,
            warmup_runs: 3,
            metrics: MetricsMask::all(),
            // Insert and query are assumed of every implementation; merge and
            // prepare are declared, so a caller asks for them by name.
            operations: OperationMask::INSERT | OperationMask::QUERY,
            query_count: None,
            threads: 1,
            merge_shards: 2,
            seed: 0,
        }
    }
}
