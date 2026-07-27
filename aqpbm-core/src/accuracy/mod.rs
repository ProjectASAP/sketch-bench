//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait a
//! benchmark consults, the [`Comparison`] it returns, the per-statistic
//! capability traits that say which sketch can be asked which question, and
//! the comparators that score each statistic.
//!
//! None of it names a sketch family. A comparator binds a *capability*
//! ([`CardinalityOps`], [`FrequencyOps`], ...), not a family, so it scores any
//! implementation that declares that capability — the ones in `sketch-bench`
//! and anybody else's alike.
//!
//! See `docs/DESIGN.md` §5.6.

use std::collections::BTreeMap;

use crate::metrics::QueryCallSample;
use crate::sketch::Sketch;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
pub mod statistic;
pub mod topk;

// The capability traits that declare which sketch answers which statistic.
pub use statistic::{CardinalityOps, FrequencyOps, QuantileOps, TopKOps};

/// Output of a single ground-truth comparison run: the comparator's named
/// scalars plus the timing of the estimate calls it issued. The runner pulls
/// `queries` / `query_wall_ns` into `RunMetrics` so query throughput lands in
/// the aggregate beside insertion throughput. Only the sketch's own estimate
/// call is timed, not the exact-truth build (HashMap / sort), so the number
/// means "ops/sec the sketch can answer".
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// Named scalars, not an opaque JSON blob.
    ///
    /// A blob could only be aggregated by code that knew each family's shape,
    /// which is why `aggregate` used to give up and publish the last run's
    /// numbers verbatim — ten repetitions reported as one. A flat
    /// `name -> f64` map folds generically through the same Welford, over the
    /// independent draws the runner makes per repetition. Comparators name
    /// their own keys, and the name must state the population the number is
    /// over (`are_top10`, `are_all`): an average relative error over heavy
    /// hitters and one over every distinct key are different numbers that the
    /// literature has published under the same word.
    pub metrics: BTreeMap<String, f64>,
    pub queries: u64,
    pub query_wall_ns: u64,
    /// Optional per-call samples — populated only when the comparator was
    /// constructed with the `record_calls` flag (set by
    /// `sketchlib bench --raw-csv`, which reproduces the retired per-family
    /// query harnesses' CSV shape). `None` otherwise, so production runs pay
    /// nothing.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

/// A comparator between a sketch's estimate and an exact answer computed from
/// the raw workload. Impls name their own keys in [`Comparison::metrics`];
/// the runner carries that map through to `RunMetrics::accuracy`.
pub trait GroundTruth<S: Sketch> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
