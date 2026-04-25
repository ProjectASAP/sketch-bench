//! Ground-truth accuracy comparators. Four built-in families
//! (frequency, cardinality, quantile, top-k) + a trait so apps
//! can plug their own.
//!
//! See `docs/DESIGN.md` §5.6.

use sketch_core::sketch::Sketch;

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
    pub json: serde_json::Value,
    pub queries: u64,
    pub query_wall_ns: u64,
}

/// A comparator between a sketch's estimate and an exact answer
/// computed from the raw workload. Concrete impls pick their
/// own JSON shape inside `Comparison.json`; the runner stuffs
/// that into `RunMetrics.accuracy` opaquely.
pub trait GroundTruth<S: Sketch> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
