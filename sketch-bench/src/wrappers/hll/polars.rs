//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use ::polars::prelude::*;
use aqpbm_core::RunError;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

/// Polars computes the exact answer and has no `(rows, cols)` to tune, so it
/// ignores the `ParamSet` and builds unconditionally. Its record carries
/// whatever config the run was given.
pub fn build_polars_cardinality(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsCardinality, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HllParams = config.parse()?;
    Ok(PolarsCardinality {
        buf: Vec::new(),
        estimate: 0.0,
    })
}

pub fn memory_polars_cardinality(sketch: &PolarsCardinality) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<i64>()
}

impl PolarsCardinality {
    pub fn estimate_distinct(&self) -> f64 {
        self.estimate
    }
}

pub fn insert_polars_cardinality(sketch: &mut PolarsCardinality, v: &i64) {
    sketch.buf.push(*v);
}

pub fn prepare_polars_cardinality(sketch: &mut PolarsCardinality) {
    let series = Column::new("v".into(), &sketch.buf);
    let df = DataFrame::new(vec![series]).expect("DataFrame::new");
    let result = df
        .lazy()
        .select([col("v").n_unique().alias("c")])
        .collect()
        .expect("polars n_unique collect");
    let c = result
        .column("c")
        .expect("c column")
        .cast(&DataType::Float64)
        .expect("cast to f64");
    sketch.estimate = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
}
