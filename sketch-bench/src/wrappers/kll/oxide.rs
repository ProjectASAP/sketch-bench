//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use aqpbm_core::accuracy::quantile::QuantileValue;
use aqpbm_core::RunError;

/// uses; `sketch_oxide` takes a `u16`. Refuse out of range by name instead of
/// truncating, which would run at a `k` other than the one requested.
fn oxide_kll(k: u32) -> Result<sketch_oxide::quantiles::KllSketch, RunError> {
    let k16 = u16::try_from(k)
        .map_err(|_| RunError::Sketch(format!("oxide KLL: k={k} exceeds the u16 its API takes")))?;
    sketch_oxide::quantiles::KllSketch::new(k16)
        .map_err(|e| RunError::Sketch(format!("oxide KLL rejected k={k16}: {e:?}")))
}

/// Generic over the item type: the inner sketch is `f64`-native, so `T = f64`
/// monomorphises `to_f64` away while `T = i64` keeps the cast — the measurement.
pub struct KllOxidePerCall<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    _item: std::marker::PhantomData<T>,
}

pub fn build_kll_oxide_per_call<T: QuantileValue>(
    config: &ParamSet,
) -> Result<KllOxidePerCall<T>, RunError> {
    let p: KllParams = config.parse()?;
    Ok(KllOxidePerCall {
        inner: oxide_kll(p.k)?,
        k: p.k,
        _item: std::marker::PhantomData,
    })
}

pub fn memory_kll_oxide_per_call<T: QuantileValue>(sketch: &KllOxidePerCall<T>) -> usize {
    kll_footprint::<f64>(sketch.k)
}

impl<T: QuantileValue> KllOxidePerCall<T> {
    /// `&mut` because that is what the library wants. No wrapper, no cell.
    pub fn estimate_quantile(&mut self, phi: f64) -> f64 {
        self.inner.quantile(phi).unwrap_or(f64::NAN)
    }
}

pub struct KllOxideCdf<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    /// `(value, cumulative_rank)` pairs, built in `prepare`.
    cdf: Option<Vec<(f64, f64)>>,
    ends: (f64, f64),
    _item: std::marker::PhantomData<T>,
}

pub fn build_kll_oxide_cdf<T: QuantileValue>(
    config: &ParamSet,
) -> Result<KllOxideCdf<T>, RunError> {
    let p: KllParams = config.parse()?;
    Ok(KllOxideCdf {
        inner: oxide_kll(p.k)?,
        k: p.k,
        cdf: None,
        ends: (f64::NAN, f64::NAN),
        _item: std::marker::PhantomData,
    })
}

pub fn memory_kll_oxide_cdf<T: QuantileValue>(sketch: &KllOxideCdf<T>) -> usize {
    kll_footprint::<f64>(sketch.k)
}

impl<T: QuantileValue> KllOxideCdf<T> {
    pub fn estimate_quantile(&mut self, phi: f64) -> f64 {
        if let Some(table) = self.cdf.as_ref() {
            let (min, max) = self.ends;
            return query_cdf(table, phi, min, max);
        }
        // `prepare` always runs before the query phase, so this is unreachable
        // through the runner. Falling back to the per-call path keeps a direct
        // caller correct instead of handing it a NaN.
        self.inner.quantile(phi).unwrap_or(f64::NAN)
    }
}

pub fn insert_kll_oxide_per_call<T: QuantileValue>(sketch: &mut KllOxidePerCall<T>, v: &T) {
    sketch.inner.update(v.to_f64());
}

pub fn merge_kll_oxide_per_call<T: QuantileValue>(
    into: &mut KllOxidePerCall<T>,
    from: &KllOxidePerCall<T>,
) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so k matches");
}

pub fn insert_kll_oxide_cdf<T: QuantileValue>(sketch: &mut KllOxideCdf<T>, v: &T) {
    sketch.inner.update(v.to_f64());
}

pub fn merge_kll_oxide_cdf<T: QuantileValue>(into: &mut KllOxideCdf<T>, from: &KllOxideCdf<T>) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so k matches");
    // A merged sketch invalidates any table cached from the pre-merge state.
    into.cdf = None;
}

pub fn prepare_kll_oxide_cdf<T: QuantileValue>(sketch: &mut KllOxideCdf<T>) {
    sketch.cdf = Some(sketch.inner.cdf());
    sketch.ends = (sketch.inner.min(), sketch.inner.max());
}

pub fn query_kll_oxide_per_call<T: QuantileValue>(s: &mut KllOxidePerCall<T>, phi: &f64) -> f64 {
    s.estimate_quantile(*phi)
}

pub fn query_kll_oxide_cdf<T: QuantileValue>(s: &mut KllOxideCdf<T>, phi: &f64) -> f64 {
    s.estimate_quantile(*phi)
}
