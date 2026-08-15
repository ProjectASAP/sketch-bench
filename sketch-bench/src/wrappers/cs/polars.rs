//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{RunError, WorkloadData, RowLabel};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use crate::wrappers::polars_shared::*;


pub fn insert_polars_frequency_cs(sketch: &mut PolarsFrequencyCs, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_frequency_cs(sketch: &mut PolarsFrequencyCs)
{
        sketch.0.finalize();
}

pub fn run_frequency_cs(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsFrequencyCs, i64, FrequencyGT, _>(
        cfg, data, params, label, width, insert_polars_frequency_cs,
        &FREQUENCY_CS_OPS,
    )
}

#[derive(Default)]
pub struct PolarsFrequencyCs(PolarsFrequencyCore);

impl InitSketch for PolarsFrequencyCs {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CountSketchParams = config.parse()?;
        Ok(Self::default())
    }
}

impl MemoryFootprint for PolarsFrequencyCs {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl PolarsFrequencyCs {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

pub const FREQUENCY_CS_OPS: SketchOps<PolarsFrequencyCs, i64, i64, u64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_frequency_cs),
    ask: |s, k| s.estimate_frequency(k),
        _item: std::marker::PhantomData};
