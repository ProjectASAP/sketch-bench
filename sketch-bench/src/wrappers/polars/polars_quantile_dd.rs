//! `dd/polars` — the same exact quantile grid, registered under DDSketch's
//! family so its relative-error rows have a control.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::accuracy::QuantileOps;
use crate::params::DdParams;

use super::PolarsQuantileCore;

/// `dd/polars` view of [`PolarsQuantileCore`].
#[derive(Default)]
pub struct PolarsQuantileDd(PolarsQuantileCore);

impl InitSketch for PolarsQuantileDd {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: DdParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsQuantileDd {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsQuantileDd {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl QuantileOps for PolarsQuantileDd {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

impl BenchImpl for PolarsQuantileDd { type Params = DdParams; const IMPL: &'static str = "polars"; }
