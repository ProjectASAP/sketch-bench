//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::SketchOps;
use crate::registry::GroundTruthCalculator;
use crate::wrappers::polars_shared::*;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;

/// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
pub fn build_polars_frequency_cms(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsFrequencyCms, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: CmsParams = config.parse()?;
    Ok(PolarsFrequencyCms::default())
}

pub fn memory_polars_frequency_cms(sketch: &PolarsFrequencyCms) -> usize {
    sketch.0.memory_bytes()
}

impl PolarsFrequencyCms {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

pub fn insert_polars_frequency_cms(sketch: &mut PolarsFrequencyCms, v: &i64) {
    sketch.0.update(v);
}

pub fn prepare_polars_frequency_cms(sketch: &mut PolarsFrequencyCms) {
    sketch.0.finalize();
}

pub const FREQUENCY_CMS_OPS: SketchOps<PolarsFrequencyCms, i64, i64, u64> = SketchOps {
    build: build_polars_frequency_cms,
    memory: memory_polars_frequency_cms,
    merge: None,
    prepare: Some(prepare_polars_frequency_cms),
    ask: |s, k| s.estimate_frequency(k),
    _item: std::marker::PhantomData,
};

pub fn run_frequency_cms(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <FrequencyGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::run::run_row::<PolarsFrequencyCms, i64, FrequencyGT, _>(
        cfg,
        req,
        &wk,
        &gt,
        insert_polars_frequency_cms,
        &FREQUENCY_CMS_OPS,
    )
}

#[derive(Default)]
pub struct PolarsFrequencyCms(PolarsFrequencyCore);
