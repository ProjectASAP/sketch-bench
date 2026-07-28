//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait, the
//! [`Comparison`] it returns, the per-statistic capability traits, and the
//! comparators. A comparator binds a *capability* ([`CardinalityOps`],
//! [`FrequencyOps`], …), not a family, so it scores any implementation that
//! declares that capability. See `docs/DESIGN.md` §5.6.

use std::collections::BTreeMap;

use crate::accumulator::Accumulator;
use crate::metrics::QueryCallSample;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
pub mod statistic;
pub mod subpopulation;
pub mod topk;

// The capability traits that declare which sketch answers which statistic.
pub use statistic::{CardinalityOps, FrequencyOps, QuantileOps, SubpopFrequencyOps, TopKOps};

/// Output of a single ground-truth comparison run: named scalars plus the
/// timing of the estimate calls issued. Only the sketch's own estimate call is
/// timed, not the exact-truth build, so it means "ops/sec the sketch answers".
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// Named scalars, not an opaque JSON blob, so they fold generically through
    /// Welford. A key must state the population it is over (`are_top10` vs
    /// `are_all`) — the literature publishes both under the same word.
    pub metrics: BTreeMap<String, f64>,
    pub queries: u64,
    pub query_wall_ns: u64,
    /// Optional per-call samples, populated only when the comparator carries
    /// the `record_calls` flag (set by `sketchlib bench --raw-csv`). `None`
    /// otherwise, so production runs pay nothing.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

/// A comparator between a sketch's estimate and an exact answer computed from
/// the raw workload. Impls name their own keys in [`Comparison::metrics`];
/// the runner carries that map through to `RunMetrics::accuracy`.
pub trait GroundTruth<S: Accumulator> {
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
