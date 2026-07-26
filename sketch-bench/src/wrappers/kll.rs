//! KLL wrappers: `oxide`, `sketchlib` (a.k.a. asap_sketchlib).
//! Quantile family: `estimate_quantile(phi) -> f64`.
//!
//! Note: `sketch_oxide::quantiles::KllSketch::quantile` takes `&mut self`
//! (the sketch sorts lazily), so the inner sketch is held in a `RefCell`.
//! `QuantileOps::estimate_quantile` is `&self` because most sketches' query
//! paths are pure reads; KLL-oxide is the one that isn't.

use std::cell::RefCell;

use crate::accuracy::quantile::QuantileValue;
use crate::accuracy::QuantileOps;
use crate::init::{BenchImpl, BuildError, InitSketch};
use crate::params::KllParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::{MergeUnsupported, Sketch};
use sketch_oxide::Mergeable as _;

// ---------- sketch_oxide KLL ----------
// `KllSketch::default()` constructs with the crate's built-in `k`
// (no `new(k)` constructor exposed through the stable surface).
// `init` accepts any requested `k` but cannot honour it — it stores
// the value only so `memory_bytes` is sensible, so `k` changes the
// reported footprint, not the sketch.
///
/// Generic over the item type. The inner sketch is `f64`-native, so `T = f64`
/// monomorphises `to_f64` to the identity and the insert path holds no
/// conversion at all; `T = i64` keeps the `as f64` it always had. That
/// difference is the measurement — before this was generic only the `i64`
/// side existed, so the conversion was unavoidable and therefore invisible.
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

impl<T: QuantileValue> Sketch for KllOxide<T> {
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.get_mut().update(v.to_f64());
    }
    fn memory_bytes(&self) -> usize {
        (self.k as usize) * std::mem::size_of::<f64>() * 4
    }

    /// Unlike HLL, KLL's merge is **lossy**: combining compactors adds error
    /// beyond a single pass over the same data, and the result depends on the
    /// merge order. That is precisely why the merge benchmark measures
    /// accuracy after merging rather than asserting it.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .borrow_mut()
            .merge(&other.inner.borrow())
            .expect("both operands built from one ParamSet, so k matches");
        Ok(())
    }
}

// ---------- asap_sketchlib KLL ----------
//
// `asap_sketchlib::KLL::quantile(q)` rebuilds the full CDF from the compactor
// levels on every call (sort + sweep over the whole buffer). For an
// apples-to-apples query throughput comparison we precompute it once in
// `finalize_for_query`; queries then collapse to a `Cdf::query` binary search.
//
// `update` deliberately does NOT invalidate that cache. `BenchRunner` calls
// `finalize_for_query` once, after the insert phase has ended, and never
// re-inserts; a per-update `RefCell::borrow()` check would be a measurable
// cost on a hot 10ns/op insert path.
///
/// Unlike the oxide and DDSketch wrappers, this one is generic *in the
/// library*: `asap_sketchlib::KLL<T: NumericalValue>` stores `T` and orders it
/// with `T::total_cmp`, so nothing is converted on either side of the axis.
/// `KLL<i64>` compares integers, `KLL<f64>` compares floats. That makes it the
/// row where the dtype axis measures the library's own choice rather than a
/// wrapper's cast.
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

impl<T> Sketch for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.update(v);
    }
    fn memory_bytes(&self) -> usize {
        (self.k as usize) * std::mem::size_of::<T>() * 4
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        // A merged sketch invalidates any CDF cached from the pre-merge state.
        *self.cdf.borrow_mut() = None;
        Ok(())
    }
    fn finalize_for_query(&mut self) {
        *self.cdf.borrow_mut() = Some(self.inner.cdf());
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
