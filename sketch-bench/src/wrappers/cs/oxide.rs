//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::SketchOps;
use crate::registry::GroundTruthCalculator;
use crate::wrappers::require_resolved_shape;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;

// No `rows` / `cols` field, for the same reason as `CmsOxide`: `init` proves the
// built table matches the request, so the sketch is the only place either
// figure is read from.
//
// One bound this row has and the Count-Min one does not: the crate floors its
// depth at 3, so the median is taken over enough estimates to be one. `rows < 3`
// is therefore unreachable, and refused by name.
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
}

pub fn build_cs_oxide(config: &ParamSet, _workers: usize) -> Result<CsOxide, BuildError> {
    let p: CountSketchParams = config.parse()?;
    let (epsilon, delta) = dims_to_err(p.rows, p.cols);
    let inner = sketch_oxide::frequency::CountSketch::new(epsilon, delta).map_err(|e| {
        BuildError(format!(
            "oxide CountSketch rejected ε={epsilon} δ={delta}: {e:?}"
        ))
    })?;
    require_resolved_shape(
        "oxide CountSketch",
        (inner.depth(), inner.width()),
        (p.rows, p.cols),
    )?;
    Ok(CsOxide { inner })
}

pub fn memory_cs_oxide(sketch: &CsOxide) -> usize {
    // Off the built sketch: the crate rounds the width up to a power of two
    // and floors the depth at 3, so at `rows=2` a request-derived figure
    // under-reports by a third. Backing store is `table: Vec<i64>`.
    sketch.inner.depth() * sketch.inner.width() * std::mem::size_of::<i64>()
}

impl CsOxide {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        // CountSketch is unbiased (median of sign·counter), so collision noise
        // can push an estimate slightly negative. Clamp to 0 for CMS-style
        // semantics — otherwise `as u64` wraps -1 into u64::MAX.
        self.inner.estimate(key).max(0) as u64
    }
}

pub fn insert_cs_oxide(sketch: &mut CsOxide, v: &i64) {
    sketch.inner.update(v, 1);
}

pub fn merge_cs_oxide(into: &mut CsOxide, from: &CsOxide) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so rows/cols match");
}

pub const OXIDE_OPS: SketchOps<CsOxide, i64, i64, u64> = SketchOps {
    build: build_cs_oxide,
    memory: memory_cs_oxide,
    merge: Some(merge_cs_oxide),
    prepare: None,
    ask: ask_cs_oxide,
    _item: std::marker::PhantomData,
};

pub fn ask_cs_oxide(sketch: &mut CsOxide, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn run_oxide(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <FrequencyGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::run::run_row::<CsOxide, i64, FrequencyGT, _>(
        cfg,
        req,
        &wk,
        &gt,
        insert_cs_oxide,
        &OXIDE_OPS,
    )
}
