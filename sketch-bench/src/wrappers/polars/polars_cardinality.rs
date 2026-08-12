//! `hll/polars` — exact distinct count via `n_unique`.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;
use aqpbm_core::accuracy::CardinalityOps;
use crate::params::HllParams;

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

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
            estimate: 0.0,
        })
    }
}

impl Accumulator for PolarsCardinality {
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn prepare(&mut self) {
        let series = Column::new("v".into(), &self.buf);
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
        self.estimate = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
    }

}

impl MemoryFootprint for PolarsCardinality {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

impl CardinalityOps for PolarsCardinality {
    fn estimate_distinct(&self) -> f64 {
        self.estimate
    }
}

impl BenchImpl for PolarsCardinality { type Params = HllParams; const IMPL: &'static str = "polars"; }
