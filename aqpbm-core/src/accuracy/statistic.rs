//! One capability trait per statistic this crate measures, each with whatever
//! query shape that statistic actually wants.
//!
//! `impl CardinalityOps for HllOxide` is where "HLL is a cardinality sketch"
//! is stated — plural, and compiler-checked, because each comparator binds
//! the capability it needs. This is nominal on purpose: the old structural
//! bound (`S: Accumulator<Query = (), Answer = f64>`) also matched UnivMon and the
//! parallel-HLL shard, which answer a stub, and only the catalog's
//! hand-maintained `scores_accuracy` column kept them out.

// ---------- cardinality ----------

/// Answers "how many distinct items have you seen".
/// Scored by [`CardinalityGT`](super::cardinality::CardinalityGT).
pub trait CardinalityOps {
    fn estimate_distinct(&self) -> f64;
}

// ---------- frequency ----------

/// Answers "how many times did this key occur".
/// Scored by [`FrequencyGT`](super::frequency::FrequencyGT).
///
/// Takes the key by reference: a frequency probe loop is timed, and the
/// wrapper should not be charged for a clone it does not need.
pub trait FrequencyOps {
    type Key;
    fn estimate_frequency(&self, key: &Self::Key) -> u64;
}

// ---------- quantile ----------

/// Answers "what value sits at this quantile".
///
/// Scored by two different rulers — [`RankErrorGT`](super::quantile::RankErrorGT)
/// measures how wrong the *rank* is, [`RelativeErrorGT`](super::quantile::RelativeErrorGT)
/// how wrong the *value* is. They share this one query interface, which is why
/// the catalog row, not the type system, is what picks between them.
pub trait QuantileOps {
    /// `phi` is a fraction in `0.0..=1.0`.
    fn estimate_quantile(&self, phi: f64) -> f64;
}

// ---------- top-k ----------

/// Answers "which k keys are heaviest, and how heavy".
/// Scored by [`TopkGT`](super::topk::TopkGT).
pub trait TopKOps {
    type Key;
    fn estimate_topk(&self, k: usize) -> Vec<(Self::Key, u64)>;
}
