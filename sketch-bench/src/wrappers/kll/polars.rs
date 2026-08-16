//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::{Body, SketchOps};
use crate::registry::GroundTruthCalculator;
use crate::wrappers::polars_shared::*;
use aqpbm_core::accuracy::quantile::RankErrorGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::request::Requirement;
use aqpbm_core::workload::WorkloadDescription;
use std::rc::Rc;

/// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
pub fn build_polars_quantile_kll(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsQuantileKll, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: KllParams = config.parse()?;
    Ok(PolarsQuantileKll::default())
}

pub fn memory_polars_quantile_kll(sketch: &PolarsQuantileKll) -> usize {
    sketch.0.memory_bytes()
}

impl PolarsQuantileKll {
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

pub fn insert_polars_quantile_kll(sketch: &mut PolarsQuantileKll, v: &i64) {
    sketch.0.update(v);
}

pub fn prepare_polars_quantile_kll(sketch: &mut PolarsQuantileKll) {
    sketch.0.finalize();
}

pub const QUANTILE_KLL_OPS: SketchOps<PolarsQuantileKll, i64, f64, f64> = SketchOps {
    build: build_polars_quantile_kll,
    memory: memory_polars_quantile_kll,
    merge: None,
    prepare: Some(prepare_polars_quantile_kll),
    ask: |s, phi| s.estimate_quantile(*phi),
    _item: std::marker::PhantomData,
};

pub fn run_quantile_kll(
    req: &Requirement,
    data: WorkloadData,
) -> Result<(WorkloadDescription, Vec<Body>), RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <RankErrorGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::ops::squares_for::<_, PolarsQuantileKll, i64, RankErrorGT, _>(
        req,
        Rc::new(wk),
        gt,
        insert_polars_quantile_kll,
        QUANTILE_KLL_OPS,
    )
}

#[derive(Default)]
pub struct PolarsQuantileKll(PolarsQuantileCore);
