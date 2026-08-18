//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::require_resolved_shape;
use aqpbm_core::RunError;

// No `rows` / `cols` field, as in `CmsOxide`: `init` proves the built table
// matches the request, so the sketch is the only place either is read from.
// One extra bound — the crate floors depth at 3, so `rows < 3` is refused.
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
}

pub fn build_cs_oxide(config: &ParamSet, _workers: usize) -> Result<CsOxide, RunError> {
    let p: CountSketchParams = config.parse()?;
    let (epsilon, delta) = dims_to_err(p.rows, p.cols);
    let inner = sketch_oxide::frequency::CountSketch::new(epsilon, delta).map_err(|e| {
        RunError::Target(format!(
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

pub fn ask_cs_oxide(sketch: &mut CsOxide, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
