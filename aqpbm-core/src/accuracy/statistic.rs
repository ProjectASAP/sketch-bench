//! One capability trait per statistic this crate measures, each with whatever
//! query shape that statistic actually wants. `impl CardinalityOps for X` is
//! where "X is a cardinality sketch" is stated — compiler-checked, and nominal
//! on purpose: a structural bound also matches sketches that answer a stub.

// ---------- cardinality ----------

/// Answers "how many distinct items have you seen".
/// Scored by [`CardinalityGT`](super::cardinality::CardinalityGT).
pub trait CardinalityOps {
    fn estimate_distinct(&self) -> f64;
}

// ---------- frequency ----------

/// Answers "how many times did this key occur", scored by
/// [`FrequencyGT`](super::frequency::FrequencyGT). Takes the key by reference:
/// the probe loop is timed, so no wrapper is charged for a needless clone.
pub trait FrequencyOps {
    type Key;
    fn estimate_frequency(&self, key: &Self::Key) -> u64;
}

// ---------- subpopulation frequency ----------

/// Answers "within the records carrying these labels, how many times did this
/// value occur", scored by
/// [`SubpopFrequencyGT`](super::subpopulation::SubpopFrequencyGT).
///
/// Note what the population is: a **(subpopulation, value) pair**, not a
/// subpopulation. A counter array under a grouped sketch counts values inside a
/// group; the size of the group itself is a different statistic, and a sketch
/// answering that declares a different capability.
pub trait SubpopFrequencyOps {
    type Value;
    /// `labels` in column order. The query carries values only, never column
    /// positions, because that is all a grouped sketch's key is.
    fn estimate_subpop_frequency(&self, labels: &[&str], value: &Self::Value) -> f64;
}

// ---------- quantile ----------

/// Answers "what value sits at this quantile". Two rulers score it — rank
/// error and relative error — and since they share this interface, the catalog
/// row picks between them, not the type system.
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
