//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use aqpbm_core::RunError;

use sketch_oxide::Sketch as OxideSketch;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

pub fn build_hll_oxide(config: &ParamSet, _workers: usize) -> Result<HllOxide, RunError> {
    let p: HllParams = config.parse()?;
    let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
        .map_err(|e| RunError::Target(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
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

pub fn ask_hll_oxide(sketch: &mut HllOxide, _: &()) -> f64 {
    sketch.estimate_distinct()
}
