//! Column shapes and the materialisation step, for tests only. [`build`] is the
//! important one: it is the `generate` → `from_table` pair production takes, so
//! a test using it exercises the real path rather than a shortcut past it.

use aqpbm_datagen::{
    ColumnItem, ColumnSpec, DataDistribution, DataGenError, TableDescription, UniformParameter,
    ZipfParameter,
};

use crate::input_dataset::{numeric, Materialised};

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
pub fn build<T: ColumnItem>(spec: &TableDescription) -> Result<Materialised<T>, DataGenError> {
    let table = spec.generate()?;
    numeric::from_table(spec, table)
}
