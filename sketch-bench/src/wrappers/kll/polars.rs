//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::quantile::{ RankErrorGT};
use aqpbm_core::cell::{RunError, WorkloadData, RowLabel};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use crate::wrappers::polars_shared::*;

impl InitSketch for PolarsQuantileKll {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: KllParams = config.parse()?;
        Ok(Self::default())
    }
}

impl MemoryFootprint for PolarsQuantileKll {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl PolarsQuantileKll {
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}


pub fn insert_polars_quantile_kll(sketch: &mut PolarsQuantileKll, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_quantile_kll(sketch: &mut PolarsQuantileKll)
{
        sketch.0.finalize();
}

pub const QUANTILE_KLL_OPS: SketchOps<PolarsQuantileKll, i64, f64, f64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_quantile_kll),
    ask: |s, phi| s.estimate_quantile(*phi),
        _item: std::marker::PhantomData};

pub fn run_quantile_kll(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsQuantileKll, i64, RankErrorGT, _>(
        cfg, data, params, label, width, insert_polars_quantile_kll,
        &QUANTILE_KLL_OPS,
    )
}

#[derive(Default)]
pub struct PolarsQuantileKll(PolarsQuantileCore);
