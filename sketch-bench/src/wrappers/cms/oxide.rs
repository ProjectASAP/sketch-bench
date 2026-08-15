//! The `sketch_oxide` implementations.
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
use crate::wrappers::require_resolved_shape;

// No `rows` / `cols` field: `init` has already proven the built table matches
// the request, so the sketch itself is the one place either figure is read
// from, and a library upgrade that changed the rounding cannot slip past.
pub struct CmsOxide {
    inner: sketch_oxide::frequency::CountMinSketch}

impl InitSketch for CmsOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        // Native API takes an error bound, not raw dimensions — translate.
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        let inner = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
            .map_err(|e| BuildError(format!("oxide CMS rejected ε={epsilon} δ={delta}: {e:?}")))?;
        require_resolved_shape(
            "oxide CMS",
            (inner.depth(), inner.width()),
            (p.rows, p.cols),
        )?;
        Ok(Self { inner })
    }
}

impl MemoryFootprint for CmsOxide {
    fn memory_bytes(&self) -> usize {
        // Read off the built sketch, not off the requested `(rows, cols)`: the
        // crate derives its width from ε and rounds it up to a power of two, so
        // `cols = 3000` allocates 4096 and a request-derived figure under-reports
        // by 27%. The counters are `table: Vec<u64>`, not 32-bit — sizing them
        // as `u32` once halved every reported CMS footprint, which made CMS look
        // twice as space-efficient as CountSketch at identical accuracy.
        self.inner.depth() * self.inner.width() * std::mem::size_of::<u64>()
    }
}

impl CmsOxide {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(key)
    }
}


pub fn insert_cms_oxide(sketch: &mut CmsOxide, v: &i64)
{
        sketch.inner.update(v);
}

pub fn merge_cms_oxide(into: &mut CmsOxide, from: &CmsOxide)
{
        into.inner
            .merge(&from.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
}

/// `cms/oxide`. Probe is a key, answer is a count — the shape `FrequencyGT` asks in.
pub const OXIDE_OPS: SketchOps<CmsOxide, i64, i64, u64> = SketchOps {
    merge: Some(merge_cms_oxide),
    prepare: None,
    ask: ask_cms_oxide,
        _item: std::marker::PhantomData};

pub fn ask_cms_oxide(sketch: &mut CmsOxide, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn run_oxide(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CmsOxide, i64, FrequencyGT, _>(cfg, data, params, label, width, insert_cms_oxide, &OXIDE_OPS)
}


