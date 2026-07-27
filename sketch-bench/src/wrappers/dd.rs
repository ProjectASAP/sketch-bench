//! DDSketch wrapper: `asap_sketchlib::DDSketch`.
//! Quantile family with relative-error guarantee `alpha`:
//! `estimate_quantile(phi) -> f64` (value at quantile).

use aqpbm_core::accuracy::quantile::QuantileValue;
use aqpbm_core::accuracy::QuantileOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::DdParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::memory_footprint::MemoryFootprint;
use asap_sketchlib::DDSketch;

/// Generic over the item type. `DDSketch` buckets by `log(value)`, so it is
/// `f64`-native: `T = f64` monomorphises the insert path down to `add`, while
/// `T = i64` keeps the widening cast. Same shape as `KllOxide`.
pub struct DdLib<T = i64> {
    inner: DDSketch,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for DdLib<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: DdParams = config.parse()?;
        Ok(Self {
            inner: DDSketch::new(p.alpha),
            _item: std::marker::PhantomData,
        })
    }
}

impl<T: QuantileValue> Accumulator for DdLib<T> {
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.add(&v.to_f64());
    }
}

impl<T: QuantileValue> MemoryFootprint for DdLib<T> {
    fn memory_bytes(&self) -> usize {
        // Best-effort estimate: DDSketch's bucket store is
        // dynamically sized; fall back to the serialised size.
        self.inner
            .serialize_to_bytes()
            .map(|b| b.len())
            .unwrap_or(0)
    }
}


// ---------- statistic membership ----------

impl<T: QuantileValue> QuantileOps for DdLib<T> {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.inner.get_value_at_quantile(phi).unwrap_or(f64::NAN)
    }
}

// ---------- catalog identity ----------

impl<T: QuantileValue> BenchImpl for DdLib<T> { type Params = DdParams; const IMPL: &'static str = "lib"; }
