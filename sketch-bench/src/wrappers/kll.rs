//! KLL wrappers: `oxide` and `sketchlib` (a.k.a. asap_sketchlib), each as two
//! rows. Quantile algorithm: `estimate_quantile(phi) -> f64`.
//!
//! Both libraries offer the **same two query paths**, and which one a row takes
//! is the thing these four rows exist to price.
//!
//! - **per-call** asks the sketch for one quantile at a time. Each call pays the
//!   cost of arranging the retained items: `sketch_oxide::KllSketch::quantile`
//!   collects every level into one weighted vector and sorts it, and
//!   `asap_sketchlib::KLL::quantile` rebuilds its CDF.
//! - **cdf** builds that arrangement once in `prepare` and answers out of it.
//!   The build lands on the finalize clock, so it shows up in
//!   `finalize_time_ms` and in `build_throughput_items_per_sec` instead of
//!   silently discounting the query rate.
//!
//! Splitting them is what makes the numbers mean something. One row per library,
//! each taking a different path, would have compared two libraries **and** two
//! query strategies in one column, and the ~500× gap that produced reads as a
//! library difference when most of it is the strategy.
//!
//! The split lives on the **algorithm** axis, as `kll-percall` and `kll-cdf`.
//! The two paths give different answers, so they are two questions, and putting
//! them here leaves the impl axis free to mean the one thing it should mean:
//! which library. Each algorithm then holds `oxide` against `lib` directly.
//!
//! `sketch_oxide`'s `quantile` and `cdf` both take `&mut self` (it sorts
//! lazily). That used to force a `RefCell` around the inner sketch, because the
//! `QuantileOps` trait declared `estimate_quantile(&self)` on everyone's behalf
//! — so these rows paid a runtime borrow check per query for a signature they
//! did not need. The ask is a closure now and takes `&mut`, so the cell is gone
//! and the library is called directly.

use crate::params::KllParams;

use aqpbm_core::accuracy::quantile::QuantileValue;
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
pub struct KllOxidePerCall<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for KllOxidePerCall<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: oxide_kll(p.k)?,
            k: p.k,
            _item: std::marker::PhantomData,
        })
    }
}


impl<T: QuantileValue> MemoryFootprint for KllOxidePerCall<T> {
    fn memory_bytes(&self) -> usize {
        kll_footprint::<f64>(self.k)
    }
}

impl<T: QuantileValue> KllOxidePerCall<T> {
    /// `&mut` because that is what the library wants. No wrapper, no cell.
    pub fn estimate_quantile(&mut self, phi: f64) -> f64 {
        self.inner.quantile(phi).unwrap_or(f64::NAN)
    }
}

// ---------- sketch_oxide KLL, CDF built once ----------

pub struct KllOxideCdf<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    /// `(value, cumulative_rank)` pairs, built in `prepare`.
    cdf: Option<Vec<(f64, f64)>>,
    ends: (f64, f64),
    _item: std::marker::PhantomData<T>,
}

impl<T: QuantileValue> InitSketch for KllOxideCdf<T> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: KllParams = config.parse()?;
        Ok(Self {
            inner: oxide_kll(p.k)?,
            k: p.k,
            cdf: None,
            ends: (f64::NAN, f64::NAN),
            _item: std::marker::PhantomData,
        })
    }
}


impl<T: QuantileValue> MemoryFootprint for KllOxideCdf<T> {
    fn memory_bytes(&self) -> usize {
        kll_footprint::<f64>(self.k)
    }
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

// ---------- asap_sketchlib KLL, CDF built once ----------

pub struct KllLibCdf<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
    cdf: Option<asap_sketchlib::sketches::kll::Cdf>,
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
            cdf: None,
        })
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
    const SUPPORTS_MERGE: bool = true;
}
impl<T: QuantileValue> BenchImpl for KllOxideCdf<T> {
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "oxide";
    const SUPPORTS_MERGE: bool = true;
    const SUPPORTS_PREPARE: bool = true;
}
impl<T> BenchImpl for KllLibPerCall<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-percall";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}
impl<T> BenchImpl for KllLibCdf<T>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
    const SUPPORTS_PREPARE: bool = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::config::SketchParams;

    fn params(k: u32) -> ParamSet {
        ParamSet::of(&KllParams { k })
    }

    fn fed<S: InitSketch>(
        k: u32,
        items: &[i64],
        insert: fn(&mut S, &i64),
        ops: aqpbm_core::ops::SketchOps<S, i64, f64, f64>,
    ) -> S {
        let mut s = S::init(&params(k)).expect("a valid k builds");
        for v in items {
            insert(&mut s, v);
        }
        ops.run_prepare(&mut s);
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
        let mut small: KllOxidePerCall<i64> = fed(50, &items, insert_kll_oxide_per_call, oxide_percall_ops::<i64>());
        let mut large: KllOxidePerCall<i64> = fed(800, &items, insert_kll_oxide_per_call, oxide_percall_ops::<i64>());
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
        let small: KllLibPerCall<i64> = fed(8, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
        let large: KllLibPerCall<i64> = fed(800, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
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
            let mut per_call: KllOxidePerCall<i64> = fed(k, &items, insert_kll_oxide_per_call, oxide_percall_ops::<i64>());
            let mut cdf: KllOxideCdf<i64> = fed(k, &items, insert_kll_oxide_cdf, oxide_cdf_ops::<i64>());
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
        let per_call: KllLibPerCall<i64> = fed(200, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
        let cdf: KllLibCdf<i64> = fed(200, &items, insert_kll_lib_cdf, lib_cdf_ops::<i64>());

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
        // Each row's own ask, exactly as the catalog writes it.
        let mut per_call = per_call;
        let mut cdf = cdf;
        let per_call_err = err(aqpbm_core::accuracy::run_probes(
            &gt,
            &|s: &mut KllLibPerCall<i64>, phi: &f64| s.estimate_quantile(*phi),
            &mut per_call,
            &items,
            false,
        ));
        let cdf_err = err(aqpbm_core::accuracy::run_probes(
            &gt,
            &|s: &mut KllLibCdf<i64>, phi: &f64| s.estimate_quantile(*phi),
            &mut cdf,
            &items,
            false,
        ));
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
        let oxide_a: KllOxidePerCall<i64> = fed(k, &items, insert_kll_oxide_per_call, oxide_percall_ops::<i64>());
        let oxide_b: KllOxideCdf<i64> = fed(k, &items, insert_kll_oxide_cdf, oxide_cdf_ops::<i64>());
        assert_eq!(oxide_a.memory_bytes(), oxide_b.memory_bytes());
        let lib_a: KllLibPerCall<i64> = fed(k, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
        let lib_b: KllLibCdf<i64> = fed(k, &items, insert_kll_lib_cdf, lib_cdf_ops::<i64>());
        assert_eq!(lib_a.memory_bytes(), lib_b.memory_bytes());
    }
}

// ---------- how this sketch is driven ----------
//
// One function per operation, per sketch. These used to be an
// `impl Accumulator for X` block, which fixed one signature for every
// implementation in the repo. As free functions each states its own
// terms, and `catalog` names them in the row's `SketchOps`.
    #[inline(always)]
pub fn insert_kll_oxide_per_call<T: QuantileValue>(sketch: &mut KllOxidePerCall<T>, v: &T)
{
        sketch.inner.update(v.to_f64());
}

pub fn merge_kll_oxide_per_call<T: QuantileValue>(into: &mut KllOxidePerCall<T>, from: &KllOxidePerCall<T>)
{
        into.inner
            .merge(&from.inner)
            .expect("both operands built from one ParamSet, so k matches");
}
    #[inline(always)]
pub fn insert_kll_oxide_cdf<T: QuantileValue>(sketch: &mut KllOxideCdf<T>, v: &T)
{
        sketch.inner.update(v.to_f64());
}

pub fn merge_kll_oxide_cdf<T: QuantileValue>(into: &mut KllOxideCdf<T>, from: &KllOxideCdf<T>)
{
        into.inner
            .merge(&from.inner)
            .expect("both operands built from one ParamSet, so k matches");
        // A merged sketch invalidates any table cached from the pre-merge state.
        into.cdf = None;
}

pub fn prepare_kll_oxide_cdf<T: QuantileValue>(sketch: &mut KllOxideCdf<T>)
{
        sketch.cdf = Some(sketch.inner.cdf());
        sketch.ends = (sketch.inner.min(), sketch.inner.max());
}
    #[inline(always)]
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
    #[inline(always)]
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

// ---------- the rows this file provides ----------
//
// One generic ops body per row, instantiated at both widths. The ask takes
// `&mut` — which is why the `RefCell` these wrappers used to carry is gone.

use aqpbm_core::accuracy::quantile::RankErrorGT;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};

pub const fn oxide_percall_ops<T: QuantileValue>() -> SketchOps<KllOxidePerCall<T>, T, f64, f64> {
    SketchOps {
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
        merge: Some(merge_kll_oxide_cdf),
        prepare: Some(prepare_kll_oxide_cdf),
        ask: ask_kll_oxide_cdf,
        _item: std::marker::PhantomData,
    }
}
pub fn ask_kll_oxide_cdf<T: QuantileValue>(s: &mut KllOxideCdf<T>, phi: &f64) -> f64 {
    s.estimate_quantile(*phi)
}

pub const fn lib_percall_ops<T>() -> SketchOps<KllLibPerCall<T>, T, f64, f64>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    SketchOps {
        merge: Some(merge_kll_lib_per_call),
        prepare: None,
        ask: ask_kll_lib_per_call,
        _item: std::marker::PhantomData,
    }
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
        _item: std::marker::PhantomData,
    }
}
pub fn ask_kll_lib_cdf<T>(s: &mut KllLibCdf<T>, phi: &f64) -> f64
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    s.estimate_quantile(*phi)
}

pub fn run_oxide_percall(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_ordered::<KllOxidePerCall<i64>, KllOxidePerCall<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        width,
        insert_kll_oxide_per_call,
        &oxide_percall_ops::<i64>(),
        insert_kll_oxide_per_call,
        &oxide_percall_ops::<f64>(),
    )
}
pub fn run_lib_percall(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_ordered::<KllLibPerCall<i64>, KllLibPerCall<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        width,
        insert_kll_lib_per_call,
        &lib_percall_ops::<i64>(),
        insert_kll_lib_per_call,
        &lib_percall_ops::<f64>(),
    )
}
pub fn run_oxide_cdf(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_ordered::<KllOxideCdf<i64>, KllOxideCdf<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        width,
        insert_kll_oxide_cdf,
        &oxide_cdf_ops::<i64>(),
        insert_kll_oxide_cdf,
        &oxide_cdf_ops::<f64>(),
    )
}
pub fn run_lib_cdf(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_ordered::<KllLibCdf<i64>, KllLibCdf<f64>, RankErrorGT, _, _>(
        cfg,
        data,
        params,
        width,
        insert_kll_lib_cdf,
        &lib_cdf_ops::<i64>(),
        insert_kll_lib_cdf,
        &lib_cdf_ops::<f64>(),
    )
}
