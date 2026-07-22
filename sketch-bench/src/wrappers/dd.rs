//! DDSketch wrapper: `asap_sketchlib::DDSketch`.
//! Quantile family with relative-error guarantee `alpha`:
//! `Query = f64` (quantile in [0, 1]), `Answer = f64` (value
//! at quantile).

use crate::accuracy::quantile::QuantileValue;
use crate::params::DdParams;
use aqpbm_core::sketch::Sketch;
use asap_sketchlib::DDSketch;

/// Generic over the item type. `DDSketch` buckets by `log(value)`, so it is
/// `f64`-native: `T = f64` monomorphises the insert path down to `add`, while
/// `T = i64` keeps the widening cast. Same shape as [`KllOxide`], for the same
/// reason.
///
/// [`KllOxide`]: crate::wrappers::kll::KllOxide
pub struct DdLib<T = i64> {
    inner: DDSketch,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> DdLib<T> {
    pub fn new(p: &DdParams) -> Self {
        Self {
            inner: DDSketch::new(p.alpha),
            _item: std::marker::PhantomData,
        }
    }
}

impl<T: QuantileValue> Sketch for DdLib<T> {
    type Item = T;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.add(&v.to_f64());
    }
    fn query(&self, q: f64) -> f64 {
        self.inner.get_value_at_quantile(q).unwrap_or(f64::NAN)
    }
    fn memory_bytes(&self) -> usize {
        // Best-effort estimate: DDSketch's bucket store is
        // dynamically sized; fall back to the serialised size.
        self.inner
            .serialize_to_bytes()
            .map(|b| b.len())
            .unwrap_or(0)
    }
}
