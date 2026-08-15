//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use ::polars::prelude::*;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.


/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64}

impl InitSketch for PolarsCardinality {
    /// Polars computes the exact answer and has no `(rows, cols)` to tune, so it
    /// ignores the `ParamSet` and builds unconditionally. Its record carries
    /// whatever config the cell was given.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: HllParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            estimate: 0.0})
    }
}

impl MemoryFootprint for PolarsCardinality {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

impl PolarsCardinality {
    pub fn estimate_distinct(&self) -> f64 {
        self.estimate
    }
}

impl BenchImpl for PolarsCardinality { type Params = HllParams; const IMPL: &'static str = "polars"; const SUPPORTS_PREPARE: bool = true; }

pub fn insert_polars_cardinality(sketch: &mut PolarsCardinality, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_polars_cardinality(sketch: &mut PolarsCardinality)
{
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
    merge: None,
    prepare: Some(prepare_polars_cardinality),
    ask: |s, _| s.estimate_distinct(),
        _item: std::marker::PhantomData};

pub fn run_cardinality(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsCardinality, i64, CardinalityGT, _>(
        cfg, data, params, width, insert_polars_cardinality,
        &CARDINALITY_OPS,
    )
}


