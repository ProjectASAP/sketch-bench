//! What a frontend asks for, as one value: a [`Requirement`], from which the
//! answer to "can this run" follows alone. See `docs/sketch-bench.md` §Input.

use crate::config::ParamSet;
use crate::metrics::{MetricsMask, OperationMask};

/// The statistic a row answers, as a *value* — one variant per statistic some
/// [`GroundTruth`](crate::accuracy::GroundTruth) can score. Data, not a bound, so
/// a registry can print it. [`Capability::None`] is measured but not scored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Capability {
    Cardinality,
    Frequency,
    Quantile,
    TopK,
    HeavyHitter,
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
            Capability::TopK => "topk",
            Capability::HeavyHitter => "heavy-hitter",
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

/// The item width a row is measured at: the one item-type choice a user still
/// makes, since an ordered row (KLL) builds at either width and every other
/// row's is fixed by its Rust type. Here because [`Requirement`] carries it.
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
/// deliberately without the dataset, because what to generate is an *answer*
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
