//! The `sketch_oxide` implementations.
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
use sketch_oxide::Sketch as OxideSketch;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8}

impl InitSketch for HllOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
            .map_err(|e| BuildError(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
        Ok(Self {
            inner,
            lg_k: p.lg_k})
    }
}

impl MemoryFootprint for HllOxide {
    fn memory_bytes(&self) -> usize {
        // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
        1usize << self.lg_k
    }
}

impl HllOxide {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl BenchImpl for HllOxide { type Params = HllParams; const IMPL: &'static str = "oxide"; const SUPPORTS_MERGE: bool = true; }

pub fn insert_hll_oxide(sketch: &mut HllOxide, v: &i64)
{
        sketch.inner.update(v);
}

pub fn merge_hll_oxide(into: &mut HllOxide, from: &HllOxide)
{
        into.inner
            .merge(&from.inner)
            .expect("both operands built from one ParamSet, so lg_k matches");
}

pub const OXIDE_OPS: SketchOps<HllOxide, i64, (), f64> = SketchOps {
    merge: Some(merge_hll_oxide),
    prepare: None,
    ask: ask_hll_oxide,
        _item: std::marker::PhantomData};

pub fn ask_hll_oxide(sketch: &mut HllOxide, _: &()) -> f64 {
    sketch.estimate_distinct()
}

pub fn run_oxide(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<HllOxide, i64, CardinalityGT, _>(cfg, data, params, width, insert_hll_oxide, &OXIDE_OPS)
}


