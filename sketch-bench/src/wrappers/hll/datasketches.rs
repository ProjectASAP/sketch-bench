//! The `datasketches` (Apache DataSketches) implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::{Body, SketchOps};
use crate::registry::GroundTruthCalculator;
use crate::wrappers::require_range;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::request::Requirement;
use aqpbm_core::workload::WorkloadDescription;
use std::rc::Rc;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: ::datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: ::datasketches::hll::HllType,
}

/// What `::datasketches::hll::HllSketch::new` accepts. It asserts rather than
/// returning an error, so the bound has to be stated on this side: the twin of
/// [`LIB_PRECISIONS`], for the library that expresses its domain as a range
/// instead of a set of types.
pub const DS_LG_K: (u8, u8) = (4, 21);

pub fn build_hll_datasketches(
    config: &ParamSet,
    _workers: usize,
) -> Result<HllDatasketches, BuildError> {
    let p: HllParams = config.parse()?;
    require_range("datasketches HLL", "lg_k", p.lg_k, DS_LG_K.0, DS_LG_K.1)?;
    let hll_type = ::datasketches::hll::HllType::Hll8;
    Ok(HllDatasketches {
        inner: ::datasketches::hll::HllSketch::new(p.lg_k, hll_type),
        lg_k: p.lg_k,
        hll_type,
    })
}

pub fn memory_hll_datasketches(sketch: &HllDatasketches) -> usize {
    // Apache datasketches packs registers per HllType:
    // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
    let m = 1usize << sketch.lg_k;
    match sketch.hll_type {
        ::datasketches::hll::HllType::Hll4 => m / 2,
        ::datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
        ::datasketches::hll::HllType::Hll8 => m,
    }
}

impl HllDatasketches {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

pub fn insert_hll_datasketches(sketch: &mut HllDatasketches, v: &i64) {
    sketch.inner.update(*v);
}

pub fn merge_hll_datasketches(into: &mut HllDatasketches, from: &HllDatasketches) {
    let mut union = ::datasketches::hll::HllUnion::new(into.lg_k);
    union.update(&into.inner);
    union.update(&from.inner);
    into.inner = union.get_result(into.hll_type);
}

pub const DATASKETCHES_OPS: SketchOps<HllDatasketches, i64, (), f64> = SketchOps {
    build: build_hll_datasketches,
    memory: memory_hll_datasketches,
    merge: Some(merge_hll_datasketches),
    prepare: None,
    ask: ask_hll_datasketches,
    _item: std::marker::PhantomData,
};

pub fn ask_hll_datasketches(sketch: &mut HllDatasketches, _: &()) -> f64 {
    sketch.estimate_distinct()
}

pub fn run_datasketches(
    req: &Requirement,
    data: WorkloadData,
) -> Result<(WorkloadDescription, Vec<Body>), RunError> {
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <CardinalityGT as GroundTruthCalculator<i64>>::build(&req.params);
    crate::ops::squares_for::<_, HllDatasketches, i64, CardinalityGT, _>(
        req,
        Rc::new(wk),
        gt,
        insert_hll_datasketches,
        DATASKETCHES_OPS,
    )
}
