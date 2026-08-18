//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::polars_shared::*;
use aqpbm_core::RunError;

/// No tunable shape: the exact baseline stores the stream itself, so it
/// ignores the config rather than refusing it.
pub fn build_polars_quantile_kll(config: &ParamSet) -> Result<PolarsQuantileKll, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: KllParams = config.parse()?;
    Ok(PolarsQuantileKll::default())
}

pub fn memory_polars_quantile_kll(sketch: &PolarsQuantileKll) -> usize {
    sketch.0.memory_bytes()
}

impl PolarsQuantileKll {
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

pub fn insert_polars_quantile_kll(sketch: &mut PolarsQuantileKll, v: &i64) {
    sketch.0.update(v);
}

pub fn query_polars_quantile_kll(sketch: &mut PolarsQuantileKll, phi: &f64) -> f64 {
    sketch.estimate_quantile(*phi)
}

pub fn prepare_polars_quantile_kll(sketch: &mut PolarsQuantileKll) {
    sketch.0.finalize();
}

#[derive(Default)]
pub struct PolarsQuantileKll(PolarsQuantileCore);
