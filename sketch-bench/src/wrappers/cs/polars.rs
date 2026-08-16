//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::{Body, SketchOps};
use crate::registry::GroundTruthCalculator;
use crate::wrappers::polars_shared::*;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::request::Requirement;
use aqpbm_core::workload::WorkloadDescription;
use std::rc::Rc;

pub fn insert_polars_frequency_cs(sketch: &mut PolarsFrequencyCs, v: &i64) {
    sketch.0.update(v);
}

pub fn prepare_polars_frequency_cs(sketch: &mut PolarsFrequencyCs) {
    sketch.0.finalize();
}

pub fn run_frequency_cs(
    req: &Requirement,
    data: WorkloadData,
) -> Result<(WorkloadDescription, Vec<Body>), RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <FrequencyGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::ops::squares_for::<_, PolarsFrequencyCs, i64, FrequencyGT, _>(
        req,
        Rc::new(wk),
        gt,
        insert_polars_frequency_cs,
        FREQUENCY_CS_OPS,
    )
}

#[derive(Default)]
pub struct PolarsFrequencyCs(PolarsFrequencyCore);

/// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
pub fn build_polars_frequency_cs(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsFrequencyCs, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: CountSketchParams = config.parse()?;
    Ok(PolarsFrequencyCs::default())
}

pub fn memory_polars_frequency_cs(sketch: &PolarsFrequencyCs) -> usize {
    sketch.0.memory_bytes()
}

impl PolarsFrequencyCs {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

pub const FREQUENCY_CS_OPS: SketchOps<PolarsFrequencyCs, i64, i64, u64> = SketchOps {
    build: build_polars_frequency_cs,
    memory: memory_polars_frequency_cs,
    merge: None,
    prepare: Some(prepare_polars_frequency_cs),
    ask: |s, k| s.estimate_frequency(k),
    _item: std::marker::PhantomData,
};
