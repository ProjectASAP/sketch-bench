//! KLL wrappers: `oxide`, `sketchlib` (a.k.a. asap_sketchlib). Quantile family:
//! `estimate_quantile(phi) -> f64`. `sketch_oxide`'s `quantile` takes
//! `&mut self` (it sorts lazily), so its inner sketch sits in a `RefCell` —
//! `QuantileOps::estimate_quantile` is `&self` for everyone else's pure reads.

use std::cell::RefCell;

use aqpbm_core::accuracy::quantile::QuantileValue;
use aqpbm_core::accuracy::QuantileOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::KllParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
use sketch_oxide::Mergeable as _;

// ---------- sketch_oxide KLL ----------
// `KllSketch::default()` uses the crate's built-in `k`; `init` stores a request
// only for `memory_bytes`, so `k` moves the footprint, not the sketch.

///
/// Generic over the item type: the inner sketch is `f64`-native, so `T = f64`
/// monomorphises `to_f64` away while `T = i64` keeps the cast — the measurement.
pub struct KllOxide<T = i64> {
    inner: RefCell<sketch_oxide::quantiles::KllSketch>,
    k: u32,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for KllOxide<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: RefCell::new(sketch_oxide::quantiles::KllSketch::default()),
            k: p.k,
            _item: std::marker::PhantomData,
        })
    }
}

impl<T: QuantileValue> Accumulator for KllOxide<T> {
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.get_mut().update(v.to_f64());
    }

    /// Unlike HLL, KLL's merge is **lossy**: combining compactors adds error and
    /// the result depends on fold order, which is why the merge benchmark
    /// measures accuracy afterwards rather than asserting it.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .borrow_mut()
            .merge(&other.inner.borrow())
            .expect("both operands built from one ParamSet, so k matches");
        Ok(())
    }
}

impl<T: QuantileValue> MemoryFootprint for KllOxide<T> {
    fn memory_bytes(&self) -> usize {
        (self.k as usize) * std::mem::size_of::<f64>() * 4
    }
}

// ---------- asap_sketchlib KLL ----------
// `KLL::quantile(q)` rebuilds the full CDF per call, so we precompute once in
// `prepare`. `update` does not invalidate it: the runner never re-inserts.

/// Generic *in the library*: `KLL<T>` stores `T` and orders it with
/// `T::total_cmp`, converting nothing — so this row's item-type axis measures
/// the library's own choice, not a wrapper's cast.
pub struct KllLib<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
    cdf: RefCell<Option<asap_sketchlib::sketches::kll::Cdf>>,
}

impl<T> InitSketch for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: asap_sketchlib::KLL::<T>::init_kll(p.k as i32),
            k: p.k,
            cdf: RefCell::new(None),
        })
    }
}

impl<T> Accumulator for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.update(v);
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        // A merged sketch invalidates any CDF cached from the pre-merge state.
        *self.cdf.borrow_mut() = None;
        Ok(())
    }
    fn prepare(&mut self) {
        *self.cdf.borrow_mut() = Some(self.inner.cdf());
    }
}

impl<T> MemoryFootprint for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue, {
    fn memory_bytes(&self) -> usize {
        (self.k as usize) * std::mem::size_of::<T>() * 4
    }
}


// ---------- statistic membership ----------

impl<T: QuantileValue> QuantileOps for KllOxide<T> {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.inner.borrow_mut().quantile(phi).unwrap_or(f64::NAN)
    }
}

impl<T> QuantileOps for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn estimate_quantile(&self, phi: f64) -> f64 {
        if let Some(cdf) = self.cdf.borrow().as_ref() {
            return cdf.query(phi);
        }
        self.inner.quantile(phi)
    }
}

// ---------- catalog identity ----------

impl<T: QuantileValue> BenchImpl for KllOxide<T> { type Params = KllParams; const IMPL: &'static str = "oxide"; }
impl<T> BenchImpl for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Params = KllParams;
    const IMPL: &'static str = "lib";
}
