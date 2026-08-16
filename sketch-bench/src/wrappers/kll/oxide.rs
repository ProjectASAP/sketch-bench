//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::ops::SketchOps;
use crate::registry::GroundTruthCalculator;
use aqpbm_core::accuracy::quantile::{QuantileValue, RankErrorGT};
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Numeric;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;

/// uses; `sketch_oxide` takes a `u16`. Refuse out of range by name instead of
/// truncating, which would run at a `k` other than the one requested.
fn oxide_kll(k: u32) -> Result<sketch_oxide::quantiles::KllSketch, BuildError> {
    let k16 = u16::try_from(k)
        .map_err(|_| BuildError(format!("oxide KLL: k={k} exceeds the u16 its API takes")))?;
    sketch_oxide::quantiles::KllSketch::new(k16)
        .map_err(|e| BuildError(format!("oxide KLL rejected k={k16}: {e:?}")))
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
    _workers: usize,
) -> Result<KllOxidePerCall<T>, BuildError> {
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
    _workers: usize,
) -> Result<KllOxideCdf<T>, BuildError> {
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

pub const fn oxide_percall_ops<T: QuantileValue>() -> SketchOps<KllOxidePerCall<T>, T, f64, f64> {
    SketchOps {
        build: build_kll_oxide_per_call,
        memory: memory_kll_oxide_per_call,
        merge: Some(merge_kll_oxide_per_call),
        prepare: None,
        ask: ask_kll_oxide_per_call,
        _item: std::marker::PhantomData,
    }
}

pub fn ask_kll_oxide_per_call<T: QuantileValue>(s: &mut KllOxidePerCall<T>, phi: &f64) -> f64 {
    s.estimate_quantile(*phi)
}

pub const fn oxide_cdf_ops<T: QuantileValue>() -> SketchOps<KllOxideCdf<T>, T, f64, f64> {
    SketchOps {
        build: build_kll_oxide_cdf,
        memory: memory_kll_oxide_cdf,
        merge: Some(merge_kll_oxide_cdf),
        prepare: Some(prepare_kll_oxide_cdf),
        ask: ask_kll_oxide_cdf,
        _item: std::marker::PhantomData,
    }
}

pub fn ask_kll_oxide_cdf<T: QuantileValue>(s: &mut KllOxideCdf<T>, phi: &f64) -> f64 {
    s.estimate_quantile(*phi)
}

pub fn run_oxide_percall(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    // The one place a runtime value still selects an item type. Each arm is its
    // own monomorphisation, with its own owning closures.
    match req.width {
        Numeric::I64 => {
            let wk = <i64 as BenchItem>::materialise(data)?;
            let gt = <RankErrorGT as GroundTruthCalculator<i64>>::build(&req.params);
            crate::run::run_row::<KllOxidePerCall<i64>, i64, RankErrorGT, _>(
                cfg,
                req,
                &wk,
                &gt,
                insert_kll_oxide_per_call,
                &oxide_percall_ops::<i64>(),
            )
        }
        Numeric::F64 => {
            let wk = <f64 as BenchItem>::materialise(data)?;
            let gt = <RankErrorGT as GroundTruthCalculator<f64>>::build(&req.params);
            crate::run::run_row::<KllOxidePerCall<f64>, f64, RankErrorGT, _>(
                cfg,
                req,
                &wk,
                &gt,
                insert_kll_oxide_per_call,
                &oxide_percall_ops::<f64>(),
            )
        }
    }
}

pub fn run_oxide_cdf(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    // The one place a runtime value still selects an item type. Each arm is its
    // own monomorphisation, with its own owning closures.
    match req.width {
        Numeric::I64 => {
            let wk = <i64 as BenchItem>::materialise(data)?;
            let gt = <RankErrorGT as GroundTruthCalculator<i64>>::build(&req.params);
            crate::run::run_row::<KllOxideCdf<i64>, i64, RankErrorGT, _>(
                cfg,
                req,
                &wk,
                &gt,
                insert_kll_oxide_cdf,
                &oxide_cdf_ops::<i64>(),
            )
        }
        Numeric::F64 => {
            let wk = <f64 as BenchItem>::materialise(data)?;
            let gt = <RankErrorGT as GroundTruthCalculator<f64>>::build(&req.params);
            crate::run::run_row::<KllOxideCdf<f64>, f64, RankErrorGT, _>(
                cfg,
                req,
                &wk,
                &gt,
                insert_kll_oxide_cdf,
                &oxide_cdf_ops::<f64>(),
            )
        }
    }
}
