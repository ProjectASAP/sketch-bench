//! Exact-algorithm baselines, organised by the *statistic* they
//! compute — not by sketch family. A single baseline is the
//! ground truth for every sketch that answers the same question,
//! so the directory mirrors that sharing explicitly:
//!
//! | folder              | statistic     | exact algorithm        | sketches that share this baseline   |
//! |---------------------|---------------|------------------------|-------------------------------------|
//! | `cardinality.rs`    | cardinality   | `HashSet<i64>` + `len` | `hll`                               |
//! | `frequency.rs`      | frequency     | `HashMap<i64, u64>`    | `cms`, `countsketch`, `elastic`     |
//! | `quantile.rs`       | quantile      | sorted `Vec<i64>`      | `kll`, `dd` (DDSketch)              |
//!
//! Each baseline implements [`sketch_core::sketch::Sketch`] so
//! the bench runner drives it on the exact same insert / query
//! path as a sketch — `sketchlib bench --sketch hll --impl
//! exact,oxide,…` produces one v1 JSONL row per variant with
//! throughput / CPU / memory / accuracy reported uniformly. The
//! accuracy comparator sees zero error against the exact impl,
//! which both (a) gives the CLI a reference point for sketch
//! tradeoffs and (b) serves as a sanity check that the
//! ground-truth path is wired correctly.
//!
//! Dispatch marks these impls as `Constraint::Unparameterized` —
//! they ignore the family's `ParamSet`, so the sweep driver runs
//! each exactly once per invocation instead of once per grid
//! config.
//!
//! The `accuracy/{cms,hll,kll,dd}/rust/src/baseline.rs` harness
//! crates delegate here for their ground-truth algorithms, so
//! adding or tuning a baseline happens in exactly one place.

pub mod cardinality;
pub mod frequency;
pub mod quantile;

pub use cardinality::ExactCardinality;
pub use frequency::ExactFrequency;
pub use quantile::ExactQuantile;

/// The statistic a sketch family answers — the canonical taxonomy used to
/// pick a ground-truth baseline.
///
/// There used to be a `Statistic::for_family(&str)` here whose doc comment
/// asked the reader to "keep it in sync with the `accuracy_kind` field on
/// each `ImplEntry`" by hand. Nothing enforced that, it had no production
/// caller — only its own tests — and it was a second copy of a fact the
/// dispatch row already states. The dispatch row is the single place a
/// family's statistic is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statistic {
    /// Distinct-item count. Baseline: [`ExactCardinality`].
    Cardinality,
    /// Per-key frequency / heavy-hitter set. Baseline:
    /// [`ExactFrequency`].
    Frequency,
    /// Rank / quantile lookup. Baseline: [`ExactQuantile`].
    Quantile,
}

impl Statistic {
    /// Human-readable label — the folder name under
    /// `sketch-bench/src/baselines/` that houses the ground-truth
    /// algorithm for this statistic.
    pub fn baseline_module(&self) -> &'static str {
        match self {
            Self::Cardinality => "cardinality",
            Self::Frequency => "frequency",
            Self::Quantile => "quantile",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
