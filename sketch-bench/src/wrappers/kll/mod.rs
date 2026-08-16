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

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// figure is `heap_bytes_net`, which the tracking allocator measures; this is
/// the derived upper bound beside it, and the two are meant to be compared.
/// One rule for all four rows, so the column answers one question.
fn kll_footprint<T>(k: u32) -> usize {
    (k as usize) * std::mem::size_of::<T>() * 4
}

/// The range `asap_sketchlib::KLL::init` keeps a `k` in. Below the floor it
/// raises `k` to `m`, above the ceiling it caps; both silently. Reproduced here
/// so the two `lib` rows refuse instead, which is the only way the `k` in the
/// record is the `k` that ran. See [`LIB_K_RANGE`]'s use in `hydra.rs`, which
/// has the same cell and the same bound.
pub const LIB_K_MIN: u32 = 8;

pub const LIB_K_MAX: u32 = 26_602;

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

#[cfg(test)]
mod tests {
    use super::oxide::*;
    use super::sketchlib::*;
    use super::*;
    use aqpbm_core::config::ParamSet;

    use aqpbm_core::accuracy::quantile::RankErrorGT;
    use aqpbm_core::config::SketchParams;

    fn params(k: u32) -> ParamSet {
        ParamSet::of(&KllParams { k })
    }

    fn fed<S>(
        k: u32,
        items: &[i64],
        insert: fn(&mut S, &i64),
        ops: crate::ops::SketchOps<S, i64, f64, f64>,
    ) -> S {
        // Through the row's own build, the same one a measurement uses.
        let mut s = (ops.build)(&params(k), 1).expect("a valid k builds");
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
        let mut small: KllOxidePerCall<i64> = fed(
            50,
            &items,
            insert_kll_oxide_per_call,
            oxide_percall_ops::<i64>(),
        );
        let mut large: KllOxidePerCall<i64> = fed(
            800,
            &items,
            insert_kll_oxide_per_call,
            oxide_percall_ops::<i64>(),
        );
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
            let Err(err) = build_kll_lib_per_call::<i64>(&params(k), 1) else {
                panic!("k={k} is clamped by the library, so it must be refused");
            };
            let err = err.to_string();
            assert!(
                err.contains(&k.to_string()) && err.contains("26602"),
                "error should name the k and the bound: {err}"
            );
            assert!(
                build_kll_lib_cdf::<i64>(&params(k), 1).is_err(),
                "cdf row at k={k}"
            );
        }
        // The ends of the range are inside it.
        for k in [LIB_K_MIN, LIB_K_MAX] {
            assert!(
                build_kll_lib_per_call::<i64>(&params(k), 1).is_ok(),
                "k={k}"
            );
        }
    }

    /// Inside the range, `k` reaches the sketch. The companion to the test
    /// above: refusing out-of-range values would be worth nothing if the
    /// in-range ones were still ignored.
    #[test]
    fn lib_honours_k() {
        let items = stream();
        let small: KllLibPerCall<i64> =
            fed(8, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
        let large: KllLibPerCall<i64> = fed(
            800,
            &items,
            insert_kll_lib_per_call,
            lib_percall_ops::<i64>(),
        );
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
        let Err(err) = build_kll_oxide_per_call::<i64>(&params(70_000), 1) else {
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
            let mut per_call: KllOxidePerCall<i64> = fed(
                k,
                &items,
                insert_kll_oxide_per_call,
                oxide_percall_ops::<i64>(),
            );
            let mut cdf: KllOxideCdf<i64> =
                fed(k, &items, insert_kll_oxide_cdf, oxide_cdf_ops::<i64>());
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
        let items = stream();
        let gt = RankErrorGT {};
        let per_call: KllLibPerCall<i64> = fed(
            200,
            &items,
            insert_kll_lib_per_call,
            lib_percall_ops::<i64>(),
        );
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
        // Each row's own ask, exactly as the registry writes it.
        let mut per_call = per_call;
        let mut cdf = cdf;
        let per_call_err = err(aqpbm_core::accuracy::run_probes(
            &gt,
            &|s: &mut KllLibPerCall<i64>, phi: &f64| s.estimate_quantile(*phi),
            &mut per_call,
            &items,
        ));
        let cdf_err = err(aqpbm_core::accuracy::run_probes(
            &gt,
            &|s: &mut KllLibCdf<i64>, phi: &f64| s.estimate_quantile(*phi),
            &mut cdf,
            &items,
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
        let oxide_a: KllOxidePerCall<i64> = fed(
            k,
            &items,
            insert_kll_oxide_per_call,
            oxide_percall_ops::<i64>(),
        );
        let oxide_b: KllOxideCdf<i64> =
            fed(k, &items, insert_kll_oxide_cdf, oxide_cdf_ops::<i64>());
        assert_eq!(
            memory_kll_oxide_per_call(&oxide_a),
            memory_kll_oxide_cdf(&oxide_b)
        );
        let lib_a: KllLibPerCall<i64> =
            fed(k, &items, insert_kll_lib_per_call, lib_percall_ops::<i64>());
        let lib_b: KllLibCdf<i64> = fed(k, &items, insert_kll_lib_cdf, lib_cdf_ops::<i64>());
        assert_eq!(memory_kll_lib_per_call(&lib_a), memory_kll_lib_cdf(&lib_b));
    }
}
