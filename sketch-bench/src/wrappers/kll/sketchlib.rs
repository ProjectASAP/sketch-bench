//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::quantile::{QuantileValue, RankErrorGT};
use aqpbm_core::cell::{RunError, WorkloadData, RowLabel};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};

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
    k: u32}

impl<T> InitSketch for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: lib_kll::<T>(p.k)?,
            k: p.k})
    }
}

impl<T> MemoryFootprint for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn memory_bytes(&self) -> usize {
        kll_footprint::<T>(self.k)
    }
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
    cdf: Option<asap_sketchlib::sketches::kll::Cdf>}

impl<T> InitSketch for KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: lib_kll::<T>(p.k)?,
            k: p.k,
            cdf: None})
    }
}

impl<T> MemoryFootprint for KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn memory_bytes(&self) -> usize {
        kll_footprint::<T>(self.k)
    }
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

pub const fn lib_percall_ops<T>() -> SketchOps<KllLibPerCall<T>, T, f64, f64>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    SketchOps {
        merge: Some(merge_kll_lib_per_call),
        prepare: None,
        ask: ask_kll_lib_per_call,
        _item: std::marker::PhantomData}
}

pub fn ask_kll_lib_per_call<T>(s: &mut KllLibPerCall<T>, phi: &f64) -> f64
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    s.estimate_quantile(*phi)
}

pub const fn lib_cdf_ops<T>() -> SketchOps<KllLibCdf<T>, T, f64, f64>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    SketchOps {
        merge: Some(merge_kll_lib_cdf),
        prepare: Some(prepare_kll_lib_cdf),
        ask: ask_kll_lib_cdf,
        _item: std::marker::PhantomData}
}

pub fn ask_kll_lib_cdf<T>(s: &mut KllLibCdf<T>, phi: &f64) -> f64
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    s.estimate_quantile(*phi)
}

pub fn run_lib_percall(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::registry::run_ordered::<KllLibPerCall<i64>, KllLibPerCall<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        label,
        width,
        insert_kll_lib_per_call,
        &lib_percall_ops::<i64>(),
        insert_kll_lib_per_call,
        &lib_percall_ops::<f64>(),
    )
}

pub fn run_lib_cdf(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    label: RowLabel,
) -> Result<Vec<BenchReport>, RunError> {
    crate::registry::run_ordered::<KllLibCdf<i64>, KllLibCdf<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        label,
        width,
        insert_kll_lib_cdf,
        &lib_cdf_ops::<i64>(),
        insert_kll_lib_cdf,
        &lib_cdf_ops::<f64>(),
    )
}


