//! Ground-truth accuracy comparators. Four built-in families
//! (frequency, cardinality, quantile, top-k) + a trait so apps
//! can plug their own.
//!
//! See `docs/DESIGN.md` §5.6.

use std::collections::BTreeMap;

use aqpbm_core::sketch::Sketch;

// Per-call query telemetry is a pure metric sample with no
// sketch-domain knowledge, so its type lives in `aqpbm-core`
// alongside `RunMetrics`. Re-exported here because the comparators
// in this module are what populate it.
pub use aqpbm_core::metrics::QueryCallSample;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
#[cfg(feature = "accuracy-topk")]
pub mod topk;

/// Output of a single ground-truth comparison run. Carries the
/// per-family JSON the comparator emits + the timing of the
/// `sketch.query(...)` calls it issued. The runner pulls
/// `queries` / `query_wall_ns` into `RunMetrics` so query
/// throughput shows up in the aggregate alongside insertion
/// throughput; only the inner `sketch.query` boundary is timed,
/// not the exact-truth build (HashMap / sort), so the number is
/// meaningful as "ops/sec the sketch can answer".
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// Named scalars, not an opaque JSON blob.
    ///
    /// A blob could only be aggregated across runs by code that knew each
    /// family's shape, which is why `aggregate` used to give up and publish
    /// the last run's numbers verbatim — ten repetitions reported as one.
    /// A flat `name -> f64` map aggregates generically: the runner runs each
    /// repetition on an independent draw of the data, and `aggregate` folds
    /// every key through the same Welford. Comparators name their own keys;
    /// the name must state the population the number is over (`are_top10`,
    /// `are_all`), because an average relative error over heavy hitters and
    /// one over every distinct key are different numbers that have been
    /// published under the same word in the literature.
    pub metrics: BTreeMap<String, f64>,
    pub queries: u64,
    pub query_wall_ns: u64,
    /// Optional per-call samples — populated only when the
    /// comparator was constructed with the `record_calls` flag
    /// (set by `sketchlib bench --raw-csv` to back the legacy
    /// per-call query CSV emitted by `throughput/{hll,kll,dd}/`).
    /// Empty otherwise so production runs pay nothing.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

/// A comparator between a sketch's estimate and an exact answer
/// computed from the raw workload. Concrete impls pick their
/// own JSON shape inside `Comparison.json`; the runner stuffs
/// that into `RunMetrics.accuracy` opaquely.
pub trait GroundTruth<S: Sketch> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
