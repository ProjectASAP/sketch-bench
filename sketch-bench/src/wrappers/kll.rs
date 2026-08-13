//! KLL wrappers: `oxide` and `lib`, each at two query paths. `kll-percall` asks
//! for one quantile per call; `kll-cdf` builds the arrangement once in `prepare`,
//! so that cost lands on the finalize clock. The path is on the algorithm axis
//! because the two answer differently — one row per library would have compared
//! libraries and query strategies in one column, and the gap is mostly path.

use std::cell::RefCell;

use crate::params::KllParams;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::accuracy::quantile::QuantileValue;
use aqpbm_core::accuracy::QuantileOps;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use sketch_oxide::Mergeable as _;

/// `k` sizes the compactors, so it bounds what the sketch retains. The real
/// figure is `heap_bytes_net`, which the tracking allocator measures; this is
/// the derived upper bound beside it, and the two are meant to be compared.
/// One rule for all four rows, so the column answers one question.
fn kll_footprint<T>(k: u32) -> usize {
    (k as usize) * std::mem::size_of::<T>() * 4
}

/// `KllParams::k` is a `u32` because that is the width the parameter vocabulary
/// uses; `sketch_oxide` takes a `u16`. Refuse out of range by name instead of
/// truncating, which would run at a `k` other than the one requested.
fn oxide_kll(k: u32) -> Result<sketch_oxide::quantiles::KllSketch, BuildError> {
    let k16 = u16::try_from(k)
        .map_err(|_| BuildError(format!("oxide KLL: k={k} exceeds the u16 its API takes")))?;
    sketch_oxide::quantiles::KllSketch::new(k16)
        .map_err(|e| BuildError(format!("oxide KLL rejected k={k16}: {e:?}")))
}

/// The range `asap_sketchlib::KLL::init` keeps a `k` in. Below the floor it
/// raises `k` to `m`, above the ceiling it caps; both silently. Reproduced here
/// so the two `lib` rows refuse instead, which is the only way the `k` in the
/// record is the `k` that ran. See [`LIB_K_RANGE`]'s use in `hydra.rs`, which
/// has the same cell and the same bound.
pub const LIB_K_MIN: u32 = 8;
pub const LIB_K_MAX: u32 = 26_602;

/// The `lib` counterpart of [`oxide_kll`]: same contract, different bound. A
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

/// Answer a quantile out of a prebuilt `(value, cumulative_rank)` table, the
/// shape `sketch_oxide::KllSketch::cdf` returns: ascending by value, with the
/// cumulative rank normalised to `[0, 1]`.
///
/// `min` / `max` are passed in and short-circuit the ends, because the per-call
/// path special-cases them too and a gratuitous divergence at `phi = 0` and
/// `phi = 1` would show up as rank error that belongs to neither path.
fn query_cdf(table: &[(f64, f64)], phi: f64, min: f64, max: f64) -> f64 {
    if table.is_empty() {
        return f64::NAN;
    }
    let phi = phi.clamp(0.0, 1.0);
    if phi == 0.0 {
        return min;
    }
    if phi == 1.0 {
        return max;
    }
    // First entry whose cumulative rank reaches `phi`. Binary search, since the
    // point of this path is that the arrangement is already done.
    let idx = table.partition_point(|(_, cum)| *cum < phi);
    table[idx.min(table.len() - 1)].0
}

// ---------- sketch_oxide KLL, per-call ----------

/// Generic over the item type: the inner sketch is `f64`-native, so `T = f64`
/// monomorphises `to_f64` away while `T = i64` keeps the cast — the measurement.
///
/// `RefCell` because `sketch_oxide`'s `quantile` and `cdf` take `&mut self` (it
/// sorts lazily), while `QuantileOps::estimate_quantile` is `&self`.
pub struct KllOxidePerCall<T = i64> {
    inner: RefCell<sketch_oxide::quantiles::KllSketch>,
    k: u32,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for KllOxidePerCall<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: RefCell::new(oxide_kll(p.k)?),
            k: p.k,
            _item: std::marker::PhantomData,
        })
    }
}

impl<T: QuantileValue> Accumulator for KllOxidePerCall<T> {
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.get_mut().update(v.to_f64());
    }

    /// Unlike HLL, KLL's merge is **lossy**: combining compactors adds error and
    /// the result depends on fold order, which is why the merge benchmark
    /// measures accuracy afterwards instead of asserting it.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .borrow_mut()
            .merge(&other.inner.borrow())
            .expect("both operands built from one ParamSet, so k matches");
        Ok(())
    }
}

impl<T: QuantileValue> MemoryFootprint for KllOxidePerCall<T> {
    fn memory_bytes(&self) -> usize {
        kll_footprint::<f64>(self.k)
    }
}

impl<T: QuantileValue> QuantileOps for KllOxidePerCall<T> {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.inner.borrow_mut().quantile(phi).unwrap_or(f64::NAN)
    }
}

// ---------- sketch_oxide KLL, CDF built once ----------

pub struct KllOxideCdf<T = i64> {
    inner: RefCell<sketch_oxide::quantiles::KllSketch>,
    k: u32,
    /// `(value, cumulative_rank)` pairs, built in `prepare`.
    cdf: RefCell<Option<Vec<(f64, f64)>>>,
    ends: RefCell<(f64, f64)>,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for KllOxideCdf<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: RefCell::new(oxide_kll(p.k)?),
            k: p.k,
            cdf: RefCell::new(None),
            ends: RefCell::new((f64::NAN, f64::NAN)),
            _item: std::marker::PhantomData,
        })
    }
}

impl<T: QuantileValue> Accumulator for KllOxideCdf<T> {
    type Item = T;
    #[inline(always)]
    fn update(&mut self, v: &T) {
        self.inner.get_mut().update(v.to_f64());
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .borrow_mut()
            .merge(&other.inner.borrow())
            .expect("both operands built from one ParamSet, so k matches");
        // A merged sketch invalidates any table cached from the pre-merge state.
        *self.cdf.borrow_mut() = None;
        Ok(())
    }

    /// The whole point of this row: arrange the retained items once, on the
    /// finalize clock, so the query path is a lookup.
    fn prepare(&mut self) {
        let sketch = self.inner.get_mut();
        let table = sketch.cdf();
        let ends = (sketch.min(), sketch.max());
        *self.cdf.borrow_mut() = Some(table);
        *self.ends.borrow_mut() = ends;
    }
}

impl<T: QuantileValue> MemoryFootprint for KllOxideCdf<T> {
    fn memory_bytes(&self) -> usize {
        kll_footprint::<f64>(self.k)
    }
}

impl<T: QuantileValue> QuantileOps for KllOxideCdf<T> {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        if let Some(table) = self.cdf.borrow().as_ref() {
            let (min, max) = *self.ends.borrow();
            return query_cdf(table, phi, min, max);
        }
        // `prepare` always runs before the query phase, so this is unreachable
        // through the runner. Falling back to the per-call path keeps a direct
        // caller correct instead of handing it a NaN.
        self.inner.borrow_mut().quantile(phi).unwrap_or(f64::NAN)
    }
}

// ---------- asap_sketchlib KLL, per-call ----------

/// Generic *in the library*: `KLL<T>` stores `T` and orders it with
/// `T::total_cmp`, converting nothing — so this row's item-type axis measures
/// the library's own choice, not a wrapper's cast.
pub struct KllLibPerCall<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
}

impl<T> InitSketch for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: lib_kll::<T>(p.k)?,
            k: p.k,
        })
    }
}

impl<T> Accumulator for KllLibPerCall<T>
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
        Ok(())
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

impl<T> QuantileOps for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    /// `KLL::quantile` rebuilds the full CDF per call. That is the cost this
    /// row exists to show, so nothing here caches it.
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.inner.quantile(phi)
    }
}

// ---------- asap_sketchlib KLL, CDF built once ----------

pub struct KllLibCdf<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
    cdf: RefCell<Option<asap_sketchlib::sketches::kll::Cdf>>,
}

impl<T> InitSketch for KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: lib_kll::<T>(p.k)?,
            k: p.k,
            cdf: RefCell::new(None),
        })
    }
}

impl<T> Accumulator for KllLibCdf<T>
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

    /// `KLL::cdf()` is the library's own build-once API, not something this
    /// wrapper synthesises. `update` does not invalidate the cache: the runner
    /// never re-inserts after `prepare`.
    fn prepare(&mut self) {
        *self.cdf.borrow_mut() = Some(self.inner.cdf());
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

impl<T> QuantileOps for KllLibCdf<T>
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
// The **algorithm** states the query path, because that is what separates these
// rows from each other and the two paths answer differently. The impl is the
// library alone. There is deliberately no bare `kll` row: that name used to
// mean one arbitrary path per library, so leaving it alive would let a stale
// caller silently receive whichever one it was.

impl<T: QuantileValue> BenchImpl for KllOxidePerCall<T> {
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-percall";
    const IMPL: &'static str = "oxide";
}
impl<T: QuantileValue> BenchImpl for KllOxideCdf<T> {
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "oxide";
}
impl<T> BenchImpl for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-percall";
    const IMPL: &'static str = "lib";
}
impl<T> BenchImpl for KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "lib";
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::config::SketchParams;

    fn params(k: u32) -> ParamSet {
        ParamSet::of(&KllParams { k })
    }

    fn fed<S>(k: u32, items: &[i64]) -> S
    where
        S: InitSketch + Accumulator<Item = i64>,
    {
        let mut s = S::init(&params(k)).expect("a valid k builds");
        for v in items {
            s.update(v);
        }
        s.prepare();
        s
    }

    fn stream() -> Vec<i64> {
        (0..20_000).map(|i| (i * 7919) % 20_000).collect()
    }

    /// The bug this row had: `k` reached `memory_bytes` but never reached the
    /// sketch, so every `k` scored the same error at a different reported
    /// footprint. Two `k` values must now produce two different sketches.
    #[test]
    fn oxide_honours_k() {
        let items = stream();
        let small: KllOxidePerCall<i64> = fed(50, &items);
        let large: KllOxidePerCall<i64> = fed(800, &items);
        let (mut differs, mut checked) = (false, 0);
        for p in 1..100 {
            let phi = p as f64 / 100.0;
            if small.estimate_quantile(phi) != large.estimate_quantile(phi) {
                differs = true;
            }
            checked += 1;
        }
        assert_eq!(checked, 99);
        assert!(
            differs,
            "k=50 and k=800 answered identically at all 99 interior grid points, \
             so k is not reaching the sketch"
        );
    }

    /// The `lib` rows refuse the `k` values their library would silently clamp.
    ///
    /// This is the bug those rows had. Below the floor, `k = 1`, `4` and `8` all
    /// built one sketch at `k = 8` while `memory_bytes` reported 32, 128 and 256
    /// bytes — an eightfold spread on the memory axis for a single measurement.
    /// Above the ceiling, `k = 40000` and `k = 65535` both built at 26602 while
    /// the footprint kept climbing past 2 MB.
    #[test]
    fn lib_refuses_a_k_the_library_would_clamp() {
        for k in [0, 1, 7, LIB_K_MAX + 1, 65_535] {
            let Err(err) = KllLibPerCall::<i64>::init(&params(k)) else {
                panic!("k={k} is clamped by the library, so it must be refused");
            };
            let err = err.to_string();
            assert!(
                err.contains(&k.to_string()) && err.contains("26602"),
                "error should name the k and the bound: {err}"
            );
            assert!(KllLibCdf::<i64>::init(&params(k)).is_err(), "cdf row at k={k}");
        }
        // The ends of the range are inside it.
        for k in [LIB_K_MIN, LIB_K_MAX] {
            assert!(KllLibPerCall::<i64>::init(&params(k)).is_ok(), "k={k}");
        }
    }

    /// Inside the range, `k` reaches the sketch. The companion to the test
    /// above: refusing out-of-range values would be worth nothing if the
    /// in-range ones were still ignored.
    #[test]
    fn lib_honours_k() {
        let items = stream();
        let small: KllLibPerCall<i64> = fed(8, &items);
        let large: KllLibPerCall<i64> = fed(800, &items);
        let differs = (1..100).any(|p| {
            let phi = p as f64 / 100.0;
            small.estimate_quantile(phi) != large.estimate_quantile(phi)
        });
        assert!(
            differs,
            "k=8 and k=800 answered identically at every interior grid point, \
             so k is not reaching the sketch"
        );
    }

    /// A `k` the crate's API cannot take is refused by name, not truncated into
    /// a run at some other `k`.
    #[test]
    fn oxide_refuses_a_k_it_cannot_represent() {
        let Err(err) = KllOxidePerCall::<i64>::init(&params(70_000)) else {
            panic!("a k past u16 must be refused, not truncated into a run at some other k");
        };
        let err = err.to_string();
        assert!(err.contains("70000"), "error should name the k: {err}");
    }

    /// For oxide the two paths differ in *cost only*: `cdf()` is the same
    /// arrangement `quantile()` rebuilds per call, so answering out of it must
    /// reproduce the per-call answer at every grid point. If it did not, this
    /// split would have bought a new comparability problem instead of removing
    /// one, and the two rows' rank-error columns could not be read together.
    #[test]
    fn the_oxide_query_paths_agree_on_the_answer() {
        let items = stream();
        for k in [100, 400] {
            let per_call: KllOxidePerCall<i64> = fed(k, &items);
            let cdf: KllOxideCdf<i64> = fed(k, &items);
            for p in 0..=100 {
                let phi = p as f64 / 100.0;
                assert_eq!(
                    per_call.estimate_quantile(phi),
                    cdf.estimate_quantile(phi),
                    "oxide paths disagree at k={k}, phi={phi}"
                );
            }
        }
    }

    /// The library's two paths **do not** agree, and that is the finding this
    /// row split exists to expose. `KLL::quantile` and `Cdf::query` are separate
    /// entry points with separate interpolation, so the CDF path buys its speed
    /// with some accuracy.
    ///
    /// Before the split, `kll/lib` was the CDF path and `kll/oxide` was the
    /// per-call path, so the panel compared the cheaper answer against the more
    /// careful one under two library names and called the difference a library
    /// difference.
    ///
    /// The bound is loose on purpose. The point is to catch the CDF path
    /// becoming badly wrong, not to pin a ratio that legitimately moves.
    #[test]
    fn the_lib_query_paths_answer_differently() {
        use aqpbm_core::accuracy::quantile::RankErrorGT;

        let items = stream();
        let gt = RankErrorGT {
        };
        let per_call: KllLibPerCall<i64> = fed(200, &items);
        let cdf: KllLibCdf<i64> = fed(200, &items);

        let differs = (0..=100).any(|p| {
            let phi = p as f64 / 100.0;
            per_call.estimate_quantile(phi) != cdf.estimate_quantile(phi)
        });
        assert!(
            differs,
            "the two lib paths answered identically everywhere; if the library \
             unified them, this row split and its comment need revisiting"
        );

        let err = |c: aqpbm_core::accuracy::Comparison| c.metrics["mean_rank_err"];
        let per_call_err = err(aqpbm_core::accuracy::run_probes(&gt, &per_call, &items, false));
        let cdf_err = err(aqpbm_core::accuracy::run_probes(&gt, &cdf, &items, false));
        assert!(
            cdf_err < per_call_err * 3.0,
            "the CDF path's rank error ({cdf_err}) is far past the per-call \
             path's ({per_call_err}); it is trading away more than speed is worth"
        );
    }

    /// Every row reports its footprint by one rule, so the memory column asks
    /// one question across the four. The measured counterpart is
    /// `heap_bytes_net`, which the tracking allocator fills in.
    #[test]
    fn all_four_rows_size_themselves_by_one_rule() {
        let k = KllParams::canonical().k;
        let items = stream();
        let oxide_a: KllOxidePerCall<i64> = fed(k, &items);
        let oxide_b: KllOxideCdf<i64> = fed(k, &items);
        assert_eq!(oxide_a.memory_bytes(), oxide_b.memory_bytes());
        let lib_a: KllLibPerCall<i64> = fed(k, &items);
        let lib_b: KllLibCdf<i64> = fed(k, &items);
        assert_eq!(lib_a.memory_bytes(), lib_b.memory_bytes());
    }
}
