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

/// A comparator between a sketch's estimate and an exact answer
/// computed from the raw workload. Concrete impls pick their
/// own `Comparison` JSON shape; the runner stuffs that into
/// `RunMetrics.accuracy` opaquely.
pub trait GroundTruth<S: Sketch> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> serde_json::Value;
}
