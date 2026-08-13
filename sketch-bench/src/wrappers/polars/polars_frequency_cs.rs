//! `countsketch/polars` — the same exact counts as `cms/polars`, registered
//! under the CountSketch family so that family has its own control.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::accuracy::FrequencyOps;
use crate::params::CountSketchParams;

use super::PolarsFrequencyCore;

/// `countsketch/polars` view of [`PolarsFrequencyCore`].
#[derive(Default)]
pub struct PolarsFrequencyCs(PolarsFrequencyCore);

impl InitSketch for PolarsFrequencyCs {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CountSketchParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsFrequencyCs {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsFrequencyCs {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl FrequencyOps for PolarsFrequencyCs {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl BenchImpl for PolarsFrequencyCs { type Params = CountSketchParams; const IMPL: &'static str = "polars"; }
