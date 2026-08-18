//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::polars_shared::*;
use aqpbm_core::RunError;

pub fn insert_polars_frequency_cs(sketch: &mut PolarsFrequencyCs, v: &i64) {
    sketch.0.update(v);
}

pub fn prepare_polars_frequency_cs(sketch: &mut PolarsFrequencyCs) {
    sketch.0.finalize();
}

#[derive(Default)]
pub struct PolarsFrequencyCs(PolarsFrequencyCore);

/// No tunable shape: the exact baseline stores the stream itself, so it
/// ignores the config rather than refusing it.
pub fn build_polars_frequency_cs(
    config: &ParamSet,
    _workers: usize,
) -> Result<PolarsFrequencyCs, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: CountSketchParams = config.parse()?;
    Ok(PolarsFrequencyCs::default())
}

pub fn memory_polars_frequency_cs(sketch: &PolarsFrequencyCs) -> usize {
    sketch.0.memory_bytes()
}

impl PolarsFrequencyCs {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}
