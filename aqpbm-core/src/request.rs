//! What a frontend asks for, as one value.
//!
//! A request used to reach the registry as seven separate arguments, which meant
//! every caller reassembled it and no caller could hold one. Here it is a single
//! [`Requirement`], and the answer to "can this run" is a function of it alone —
//! see `sketch_bench::registry::resolve`. `docs/sketch-bench.md` §Input.

use crate::config::ParamSet;
use crate::metrics::{MetricsMask, OperationMask};

/// The statistic a row answers, as a *value*.
///
/// One variant per capability trait in [`crate::accuracy::statistic`], and the
/// two lists are meant to stay in step: a capability that a row can be scored
/// under is one that some `GroundTruth` knows how to compare. Naming it here as
/// data is what lets a registry *print* what a row does and refuse a comparator
/// it cannot answer, neither of which a trait bound can do.
///
/// [`Capability::None`] is a row that is measured but not scored — timed only,
/// no query. It is a real answer, not a missing one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Capability {
    Cardinality,
    Frequency,
    Quantile,
    SubpopCardinality,
    SubpopFrequency,
    SubpopQuantile,
    #[default]
    None,
}

impl Capability {
    /// The name this capability carries in a listing and in an error message.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Cardinality => "cardinality",
            Capability::Frequency => "frequency",
            Capability::Quantile => "quantile",
            Capability::SubpopCardinality => "subpop-cardinality",
            Capability::SubpopFrequency => "subpop-frequency",
            Capability::SubpopQuantile => "subpop-quantile",
            Capability::None => "none",
        }
    }

    /// Whether a comparator can score a row holding this capability. False only
    /// for [`Capability::None`], so a caller asks this instead of matching.
    pub fn scores(self) -> bool {
        !matches!(self, Capability::None)
    }
}

/// The item width a row is measured at.
///
/// The one item-type choice a user still makes: an ordered row (KLL) builds at
/// either width, while every other row's item type is fixed by its Rust type —
/// so a registry can refuse before generating anything.
///
/// It lives here rather than beside the registry because [`Requirement`] carries
/// it, and core must be able to name every field of a request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Numeric {
    #[default]
    I64,
    F64,
}

impl Numeric {
    /// The `--dtype` spelling, which is also the `data_type` a description gives
    /// the value column.
    pub fn name(self) -> &'static str {
        match self {
            Numeric::I64 => "i64",
            Numeric::F64 => "f64",
        }
    }
}

/// One measurement request: which row, built how, measured over what.
///
/// Everything a registry needs to answer "can this run, and if so how" —
/// deliberately without the workload, because what to generate is an *answer*
/// (the row's item type) rather than part of the question.
#[derive(Clone, Debug)]
pub struct Requirement {
    /// Matched against a row's algorithm exactly: one invocation is one cell.
    pub algorithm: String,
    /// The implementing library, and only that.
    pub impl_name: String,
    /// Construction parameters, already parsed into the row's vocabulary.
    pub params: ParamSet,
    /// What the metrics are taken over.
    pub operations: OperationMask,
    /// What is measured.
    pub metrics: MetricsMask,
    /// Which item width to build at.
    pub width: Numeric,
    /// Worker threads the parallel rows use. A run knob that reaches the row,
    /// so it travels with the request rather than in a whole-run config a row
    /// has no business reading.
    pub workers: usize,
    /// How many shards a merge measurement folds. A knob on the measurement,
    /// not a selector: it is read only when `operations` names merge, and the
    /// record reports the value that ran.
    pub merge_shards: usize,
    /// A named comparator, or `None` for the row's default.
    pub comparator: Option<String>,
}
