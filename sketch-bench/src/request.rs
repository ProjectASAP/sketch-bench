//! What a frontend asks for, as one value: a [`Requirement`], from which the
//! answer to "can this run" follows alone. See `docs/sketch-bench.md` §Input.

use crate::params::ParamSet;

/// The statistic a row answers, as a *value* — one variant per statistic some
/// [`GroundTruth`](aqpbm_core::GroundTruth) can score. Data, not a bound, so
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
    SubpopCdf,
    SubpopL1Norm,
    SubpopL2Norm,
    SubpopEntropy,
    SubpopSum,
    KeyedCardinality,
    KeyedL1Norm,
    KeyedL2Norm,
    KeyedEntropy,
    SumOrCount,
    Min,
    Max,
    RateOrIncrease,
    KeySet,
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
            Capability::SubpopCdf => "subpop-cdf",
            Capability::SubpopL1Norm => "subpop-l1-norm",
            Capability::SubpopL2Norm => "subpop-l2-norm",
            Capability::SubpopEntropy => "subpop-entropy",
            Capability::SubpopSum => "subpop-sum",
            Capability::KeyedCardinality => "keyed-cardinality",
            Capability::KeyedL1Norm => "keyed-l1-norm",
            Capability::KeyedL2Norm => "keyed-l2-norm",
            Capability::KeyedEntropy => "keyed-entropy",
            Capability::SumOrCount => "sum-or-count",
            Capability::Min => "min",
            Capability::Max => "max",
            Capability::RateOrIncrease => "rate-or-increase",
            Capability::KeySet => "key-set",
            Capability::None => "none",
        }
    }

    /// Whether a comparator can score a row holding this capability. False only
    /// for [`Capability::None`], so a caller asks this instead of matching.
    pub fn scores(self) -> bool {
        !matches!(self, Capability::None)
    }
}

/// The item type a row is measured at
/// Here because [`Requirement`] carries it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dtype {
    #[default]
    I64,
    U64,
    F64,
    Str,
}

impl Dtype {
    /// The `--dtype` spelling, which is also the `data_type` a description gives
    /// the value column.
    pub fn name(self) -> &'static str {
        match self {
            Dtype::I64 => "i64",
            Dtype::U64 => "u64",
            Dtype::F64 => "f64",
            Dtype::Str => "string",
        }
    }

    /// Read the `--dtype` spelling back. `None` for anything `aqpbm-datagen`
    /// cannot render.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "i64" => Some(Dtype::I64),
            "u64" => Some(Dtype::U64),
            "f64" => Some(Dtype::F64),
            "string" => Some(Dtype::Str),
            _ => None,
        }
    }
}

/// Everything a registry needs to answer "can this run, and if so how"
#[derive(Clone, Debug)]
pub struct Requirement {
    /// Matched against a row's variant exactly.
    pub variant: String,
    /// The implementing library, and only that.
    pub library: String,
    /// Construction parameters, already parsed into the row's vocabulary.
    pub params: ParamSet,
    /// Which item width to build at.
    pub width: Dtype,
    /// Worker threads the parallel rows use
    pub workers: usize,
    /// How many shards a merge measurement folds
    pub merge_shards: usize,
    /// The label columns a grouped (`hydra-*`) row asks and scores, ascending
    pub group_columns: Vec<usize>,
    /// Where a grouped row writes each scored group's error, if anywhere
    pub per_group_out: Option<std::path::PathBuf>,
    /// A named comparator, or `None` for the row's default.
    pub comparator: Option<String>,
    /// Measured runs, and the warm-ups run and discarded before them
    pub runs: usize,
    pub warmup_runs: usize,
}
