//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use aqpbm_core::accuracy::quantile::QuantileValue;
use aqpbm_core::config::ParamSet;

/// `k` this library cannot hold is an error naming both, not a run at some
/// other `k` reported as the one asked for.
fn lib_kll<T>(k: u32) -> Result<asap_sketchlib::KLL<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue,
{
    if !(LIB_K_MIN..=LIB_K_MAX).contains(&k) {
        return Err(BuildError(format!(
            "asap KLL: k={k} outside [{LIB_K_MIN}, {LIB_K_MAX}]; the library clamps \
             to that range, so any other k would run at a value this record does not name"
        )));
    }
    Ok(asap_sketchlib::KLL::<T>::init_kll(k as i32))
}

/// Generic *in the library*: `KLL<T>` stores `T` and orders it with
/// `T::total_cmp`, converting nothing — so this row's item-type axis measures
/// the library's own choice, not a wrapper's cast.
pub struct KllLibPerCall<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
}

pub fn build_kll_lib_per_call<T>(
    config: &ParamSet,
    _workers: usize,
) -> Result<KllLibPerCall<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibPerCall {
        inner: lib_kll::<T>(p.k)?,
        k: p.k,
    })
}

pub fn memory_kll_lib_per_call<T>(sketch: &KllLibPerCall<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    kll_footprint::<T>(sketch.k)
}

impl<T> KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    /// `KLL::quantile` rebuilds the full CDF per call. That is the cost this
    /// row exists to show, so nothing here caches it.
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        self.inner.quantile(phi)
    }
}

pub struct KllLibCdf<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
    cdf: Option<asap_sketchlib::sketches::kll::Cdf>,
}

pub fn build_kll_lib_cdf<T>(config: &ParamSet, _workers: usize) -> Result<KllLibCdf<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibCdf {
        inner: lib_kll::<T>(p.k)?,
        k: p.k,
        cdf: None,
    })
}

pub fn memory_kll_lib_cdf<T>(sketch: &KllLibCdf<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    kll_footprint::<T>(sketch.k)
}

impl<T> KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        if let Some(cdf) = self.cdf.as_ref() {
            return cdf.query(phi);
        }
        self.inner.quantile(phi)
    }
}

pub fn insert_kll_lib_per_call<T>(sketch: &mut KllLibPerCall<T>, v: &T)
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    sketch.inner.update(v);
}

pub fn merge_kll_lib_per_call<T>(into: &mut KllLibPerCall<T>, from: &KllLibPerCall<T>)
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    into.inner.merge(&from.inner);
}

pub fn insert_kll_lib_cdf<T>(sketch: &mut KllLibCdf<T>, v: &T)
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    sketch.inner.update(v);
}

pub fn merge_kll_lib_cdf<T>(into: &mut KllLibCdf<T>, from: &KllLibCdf<T>)
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    into.inner.merge(&from.inner);
    // A merged sketch invalidates any CDF cached from the pre-merge state.
    into.cdf = None;
}

pub fn prepare_kll_lib_cdf<T>(sketch: &mut KllLibCdf<T>)
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    sketch.cdf = Some(sketch.inner.cdf());
}

pub fn ask_kll_lib_per_call<T>(s: &mut KllLibPerCall<T>, phi: &f64) -> f64
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    s.estimate_quantile(*phi)
}

pub fn ask_kll_lib_cdf<T>(s: &mut KllLibCdf<T>, phi: &f64) -> f64
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    s.estimate_quantile(*phi)
}
