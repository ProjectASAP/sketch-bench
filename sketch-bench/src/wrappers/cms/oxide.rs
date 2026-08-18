//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::require_resolved_shape;
use aqpbm_core::RunError;

// No `rows` / `cols` field: `init` has already proven the built table matches
// the request, so the sketch itself is the one place either figure is read
// from, and a library upgrade that changed the rounding cannot slip past.
pub struct CmsOxide {
    inner: sketch_oxide::frequency::CountMinSketch,
}

pub fn build_cms_oxide(config: &ParamSet, _workers: usize) -> Result<CmsOxide, RunError> {
    let p: CmsParams = config.parse()?;
    // Native API takes an error bound, not raw dimensions — translate.
    let (epsilon, delta) = dims_to_err(p.rows, p.cols);
    let inner = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta).map_err(|e| {
        RunError::Target(format!("oxide CMS rejected ε={epsilon} δ={delta}: {e:?}"))
    })?;
    require_resolved_shape(
        "oxide CMS",
        (inner.depth(), inner.width()),
        (p.rows, p.cols),
    )?;
    Ok(CmsOxide { inner })
}

pub fn memory_cms_oxide(sketch: &CmsOxide) -> usize {
    // Read off the built sketch, not the requested `(rows, cols)`: the crate
    // rounds width up to a power of two, so `cols = 3000` allocates 4096. The
    // counters are `table: Vec<u64>`, not 32-bit — size them as `u64`.
    sketch.inner.depth() * sketch.inner.width() * std::mem::size_of::<u64>()
}

impl CmsOxide {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(key)
    }
}

pub fn insert_cms_oxide(sketch: &mut CmsOxide, v: &i64) {
    sketch.inner.update(v);
}

pub fn merge_cms_oxide(into: &mut CmsOxide, from: &CmsOxide) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so rows/cols match");
}

pub fn ask_cms_oxide(sketch: &mut CmsOxide, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
