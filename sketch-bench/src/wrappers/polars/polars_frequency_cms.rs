//! `cms/polars` — exact per-key counts, `group_by(v).agg(len)`.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::accuracy::FrequencyOps;
use crate::params::CmsParams;

use super::PolarsFrequencyCore;

/// `cms/polars` view of [`PolarsFrequencyCore`].
#[derive(Default)]
pub struct PolarsFrequencyCms(PolarsFrequencyCore);

impl InitSketch for PolarsFrequencyCms {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CmsParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsFrequencyCms {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsFrequencyCms {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl FrequencyOps for PolarsFrequencyCms {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl BenchImpl for PolarsFrequencyCms { type Params = CmsParams; const IMPL: &'static str = "polars"; }
