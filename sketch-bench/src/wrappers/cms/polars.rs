//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
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

impl InitSketch for PolarsFrequencyCms {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CmsParams = config.parse()?;
        Ok(Self::default())
    }
}

impl MemoryFootprint for PolarsFrequencyCms {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl PolarsFrequencyCms {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}


pub fn insert_polars_frequency_cms(sketch: &mut PolarsFrequencyCms, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_frequency_cms(sketch: &mut PolarsFrequencyCms)
{
        sketch.0.finalize();
}

pub const FREQUENCY_CMS_OPS: SketchOps<PolarsFrequencyCms, i64, i64, u64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_frequency_cms),
    ask: |s, k| s.estimate_frequency(k),
        _item: std::marker::PhantomData};

pub fn run_frequency_cms(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsFrequencyCms, i64, FrequencyGT, _>(
        cfg, data, params, label, width, insert_polars_frequency_cms,
        &FREQUENCY_CMS_OPS,
    )
}

#[derive(Default)]
pub struct PolarsFrequencyCms(PolarsFrequencyCore);
