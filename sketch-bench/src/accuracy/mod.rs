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
    /// Optional per-call samples — populated only when the
    /// comparator was constructed with the `record_calls` flag
    /// (set by `sketchlib bench --raw-csv` to back the legacy
    /// per-call query CSV emitted by `throughput/{hll,kll,dd}/`).
    /// Empty otherwise so production runs pay nothing.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

/// One row of per-call query telemetry. Mirrors the columns the
/// legacy `throughput/{hll,kll,dd}/rust/src/bin/query.rs` emit
/// per call: a strictly-monotonic 1-based call index, the
/// timed `sketch.query()` wall, the sketch's answer (cast to
/// `f64`), and — for quantile families — the percentile being
/// queried plus the outer repeat number.
#[derive(Debug, Clone, Copy)]
pub struct QueryCallSample {
    pub call_index: usize,
    pub nanoseconds: u64,
    pub estimate: f64,
    /// Percentile being queried (0..=100 fraction). NaN for
    /// cardinality / frequency families.
    pub percentile: f64,
    /// Outer "repeat" index used by KLL / DD legacy harnesses
    /// (each run sweeps the percentile array `REPEATS_PER_RUN`
    /// times to thicken the sample). 0 for the families that
    /// don't repeat.
    pub repeat: usize,
}

/// A comparator between a sketch's estimate and an exact answer
/// computed from the raw workload. Concrete impls pick their
/// own JSON shape inside `Comparison.json`; the runner stuffs
/// that into `RunMetrics.accuracy` opaquely.
pub trait GroundTruth<S: Sketch> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
