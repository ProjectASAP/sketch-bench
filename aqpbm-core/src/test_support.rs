//! Column shapes and the materialisation step, for tests only.
//!
//! These live here rather than in one file's `mod tests` because the same two
//! column shapes are needed by `dataset::description`, `dataset::numeric`,
//! `dataset::load` and `binfile` — four modules that each test one half of the
//! same round trip.
//!
//! [`build`] is the important one. Core deliberately has no
//! `NumericDataset::generate`: production always goes
//! `DatasetSpec::generate_at` → `TableDescription::generate` → `from_table`,
//! and a convenience constructor that skipped the middle step would let the
//! tests pass over the path that actually runs. This helper *is* those two
//! steps, so a test taking it is exercising the real one.

use aqpbm_datagen::{
    ColumnItem, ColumnSpec, DataDistribution, DataGenError, TableDescription, UniformParameter,
    ZipfParameter,
};

use crate::dataset::NumericDataset;

/// A zipfian column over ranks `[1, cardinality]`, rendered at `data_type`.
pub fn zipf_column(cardinality: u64, s: f64, seed: u64, data_type: &str) -> ColumnSpec {
    ColumnSpec {
        distribution: DataDistribution::Zipf(ZipfParameter {
            skewness: s,
            population_size: cardinality,
            seed,
        }),
        shift: None,
        cardinality: None,
        special_rule: aqpbm_datagen::RULE_NONE,
        data_type: data_type.into(),
        string: None,
    }
}

/// A uniform column over `[0, cardinality)`, rendered at `data_type`. The same
/// shape `aqpbm-cli` builds for `--dataset uniform` (`main.rs`).
pub fn uniform_column(cardinality: u64, seed: u64, data_type: &str) -> ColumnSpec {
    ColumnSpec {
        distribution: DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: cardinality as f64,
            seed,
        }),
        shift: None,
        cardinality: None,
        special_rule: aqpbm_datagen::RULE_NONE,
        data_type: data_type.into(),
        string: None,
    }
}

/// Generate `spec` and materialise it at `T` — the two steps production takes,
/// in the order it takes them.
pub fn build<T: ColumnItem>(spec: &TableDescription) -> Result<NumericDataset<T>, DataGenError> {
    let table = spec.generate()?;
    NumericDataset::from_table(spec, table)
}
