//! KLL wrappers: `oxide`, `sketchlib` (a.k.a. asap_sketchlib).
//! Quantile family: `Query = f64` (quantile in [0, 1]),
//! `Answer = f64` (value at quantile).
//!
//! Note: `sketch_oxide::quantiles::KllSketch::quantile` takes
//! `&mut self` (the sketch sorts lazily). We wrap the inner in
//! `RefCell` so the `Sketch::query(&self, ...)` contract still
//! works — the trait is intentionally `&self` since most
//! sketches' query paths are pure reads; KLL-oxide is the one
//! that isn't.

use std::cell::RefCell;

use crate::params::KllParams;
use sketch_bench::accuracy::quantile::QuantileValue;
use sketch_core::sketch::{MergeUnsupported, Sketch};
use sketch_oxide::Mergeable as _;

// ---------- sketch_oxide KLL ----------
// `KllSketch::default()` constructs with the crate's built-in `k`
// (no `new(k)` constructor exposed through the stable surface).
// We store the requested `k` so `memory_bytes` is sensible; the
// sweep driver marks oxide KLL as fixed-shape in the dispatch.
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

impl<T: QuantileValue> KllOxide<T> {
    pub fn new(p: &KllParams) -> Self {
        Self {
            inner: RefCell::new(sketch_oxide::quantiles::KllSketch::default()),
            k: p.k,
            _item: std::marker::PhantomData,
        }
    }
}

impl<T: QuantileValue> Sketch for KllOxide<T> {
    type Item = T;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.get_mut().update(v.to_f64());
    }
    fn query(&self, q: f64) -> f64 {
        self.inner.borrow_mut().quantile(q).unwrap_or(f64::NAN)
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
// `asap_sketchlib::KLL::quantile(q)` rebuilds the full CDF from
// the compactor levels on every call (sort + sweep over the
// whole buffer). For an apples-to-apples query throughput
// comparison we precompute the CDF once in `finalize_for_query`
// and cache it; queries then collapse to a `Cdf::query` binary
// search.
//
// `update` deliberately does NOT invalidate the cached CDF —
// `BenchRunner` only ever calls `finalize_for_query` once, after
// the insert phase has ended, and never re-inserts afterwards. A
// per-update RefCell::borrow() check is a measurable cost on a
// hot 10ns/op insert path; we drop it on the throughput path.
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

impl<T: asap_sketchlib::common::numerical::NumericalValue> KllLib<T> {
    pub fn new(p: &KllParams) -> Self {
        Self {
            inner: asap_sketchlib::KLL::<T>::init_kll(p.k as i32),
            k: p.k,
            cdf: RefCell::new(None),
        }
    }
}

impl<T> Sketch for KllLib<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Item = T;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.update(v);
    }
    fn query(&self, q: f64) -> f64 {
        if let Some(cdf) = self.cdf.borrow().as_ref() {
            return cdf.query(q);
        }
        self.inner.quantile(q)
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
