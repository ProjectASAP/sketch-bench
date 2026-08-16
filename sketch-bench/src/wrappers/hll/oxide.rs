//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::SketchOps;
use crate::registry::GroundTruthCalculator;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;
use sketch_oxide::Sketch as OxideSketch;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

pub fn build_hll_oxide(config: &ParamSet, _workers: usize) -> Result<HllOxide, BuildError> {
    let p: HllParams = config.parse()?;
    let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
        .map_err(|e| BuildError(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
    Ok(HllOxide {
        inner,
        lg_k: p.lg_k,
    })
}

pub fn memory_hll_oxide(sketch: &HllOxide) -> usize {
    // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
    1usize << sketch.lg_k
}

impl HllOxide {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

pub fn insert_hll_oxide(sketch: &mut HllOxide, v: &i64) {
    sketch.inner.update(v);
}

pub fn merge_hll_oxide(into: &mut HllOxide, from: &HllOxide) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so lg_k matches");
}

pub const OXIDE_OPS: SketchOps<HllOxide, i64, (), f64> = SketchOps {
    build: build_hll_oxide,
    memory: memory_hll_oxide,
    merge: Some(merge_hll_oxide),
    prepare: None,
    ask: ask_hll_oxide,
    _item: std::marker::PhantomData,
};

pub fn ask_hll_oxide(sketch: &mut HllOxide, _: &()) -> f64 {
    sketch.estimate_distinct()
}

pub fn run_oxide(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <CardinalityGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::run::run_row::<HllOxide, i64, CardinalityGT, _>(
        cfg,
        req,
        &wk,
        &gt,
        insert_hll_oxide,
        &OXIDE_OPS,
    )
}
