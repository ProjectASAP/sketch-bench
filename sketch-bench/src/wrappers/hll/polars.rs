//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::SketchOps;
use crate::registry::GroundTruthCalculator;
use ::polars::prelude::*;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

/// Polars computes the exact answer and has no `(rows, cols)` to tune, so it
/// ignores the `ParamSet` and builds unconditionally. Its record carries
/// whatever config the cell was given.
pub fn build_polars_cardinality(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsCardinality, BuildError> {
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

pub const CARDINALITY_OPS: SketchOps<PolarsCardinality, i64, (), f64> = SketchOps {
    build: build_polars_cardinality,
    memory: memory_polars_cardinality,
    merge: None,
    prepare: Some(prepare_polars_cardinality),
    ask: |s, _| s.estimate_distinct(),
    _item: std::marker::PhantomData,
};

pub fn run_cardinality(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <CardinalityGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::run::run_row::<PolarsCardinality, i64, CardinalityGT, _>(
        cfg,
        req,
        &wk,
        &gt,
        insert_polars_cardinality,
        &CARDINALITY_OPS,
    )
}
