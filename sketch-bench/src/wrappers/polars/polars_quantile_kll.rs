//! `kll-cdf/polars` — exact 101-point quantile grid. Registered on `kll-cdf`
//! and not a base algorithm, because every KLL row states a query path and this
//! one arranges its answer in `prepare`, which is what `kll-cdf` names.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::accuracy::QuantileOps;
use crate::params::KllParams;

use super::PolarsQuantileCore;

/// `kll/polars` view of [`PolarsQuantileCore`].
#[derive(Default)]
pub struct PolarsQuantileKll(PolarsQuantileCore);

impl InitSketch for PolarsQuantileKll {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: KllParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsQuantileKll {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsQuantileKll {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl QuantileOps for PolarsQuantileKll {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

impl BenchImpl for PolarsQuantileKll {
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "polars";
}
