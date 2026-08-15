//! The `datasketches` (Apache DataSketches) implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use crate::wrappers::require_range;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.


// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: ::datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: ::datasketches::hll::HllType}

/// What `::datasketches::hll::HllSketch::new` accepts. It asserts rather than
/// returning an error, so the bound has to be stated on this side: the twin of
/// [`LIB_PRECISIONS`], for the library that expresses its domain as a range
/// instead of a set of types.
pub const DS_LG_K: (u8, u8) = (4, 21);

impl InitSketch for HllDatasketches {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        require_range("datasketches HLL", "lg_k", p.lg_k, DS_LG_K.0, DS_LG_K.1)?;
        let hll_type = ::datasketches::hll::HllType::Hll8;
        Ok(Self {
            inner: ::datasketches::hll::HllSketch::new(p.lg_k, hll_type),
            lg_k: p.lg_k,
            hll_type})
    }
}

impl MemoryFootprint for HllDatasketches {
    fn memory_bytes(&self) -> usize {
        // Apache datasketches packs registers per HllType:
        // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
        let m = 1usize << self.lg_k;
        match self.hll_type {
            ::datasketches::hll::HllType::Hll4 => m / 2,
            ::datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
            ::datasketches::hll::HllType::Hll8 => m}
    }
}

impl HllDatasketches {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl BenchImpl for HllDatasketches { type Params = HllParams; const IMPL: &'static str = "datasketches"; const SUPPORTS_MERGE: bool = true; }

pub fn insert_hll_datasketches(sketch: &mut HllDatasketches, v: &i64)
{
        sketch.inner.update(*v);
}

pub fn merge_hll_datasketches(into: &mut HllDatasketches, from: &HllDatasketches)
{
        let mut union = ::datasketches::hll::HllUnion::new(into.lg_k);
        union.update(&into.inner);
        union.update(&from.inner);
        into.inner = union.get_result(into.hll_type);
}

pub const DATASKETCHES_OPS: SketchOps<HllDatasketches, i64, (), f64> = SketchOps {
    merge: Some(merge_hll_datasketches),
    prepare: None,
    ask: ask_hll_datasketches,
        _item: std::marker::PhantomData};

pub fn ask_hll_datasketches(sketch: &mut HllDatasketches, _: &()) -> f64 {
    sketch.estimate_distinct()
}

pub fn run_datasketches(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<HllDatasketches, i64, CardinalityGT, _>(
        cfg, data, params, width, insert_hll_datasketches,
        &DATASKETCHES_OPS,
    )
}


