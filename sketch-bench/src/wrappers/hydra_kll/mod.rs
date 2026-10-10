//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{
    check_grid, grid_overhead_bytes, label_columns, labels, new_hydra, query, update,
};

pub(crate) use crate::wrappers::kll::kll_lib_bytes as kll_cell_bytes;
#[cfg(test)]
use crate::wrappers::kll::{
    kll_lib_slots as kll_cell_slots, KLL_LIB_MAX_CACHEABLE_K, KLL_LIB_MAX_LEVELS, KLL_LIB_MIN_LEVEL,
};

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;

    fn built_kll() -> HydraKll {
        build_hydra_kll(
            &ParamSet::of(&HydraKllParams {
                rows: 3,
                cols: 64,
                cell_k: 200,
            }),
            2,
        )
        .expect("canonical dimensions build")
    }

    fn frecord(key: &str, value: f64) -> (String, f64) {
        (key.to_string(), value)
    }

    fn fed_kll(sketch: &mut HydraKll, r: &(String, f64)) {
        update(
            &mut sketch.inner,
            &r.0,
            &asap_sketchlib::DataInput::F64(r.1),
            "hydra-kll",
        );
    }

    #[test]
    fn canonical_params_build() {
        assert!(build_hydra_kll(&ParamSet::of(&HydraKllParams::canonical()), 2).is_ok());
    }

    /// The statistic is ordered and taken inside a group, so the median of one
    /// group must not be pulled by another group's values.
    #[test]
    fn kll_quantiles_are_taken_inside_the_group() {
        let mut h = built_kll();
        for v in 1..=101 {
            fed_kll(&mut h, &frecord("a;x", v as f64));
        }
        for _ in 0..500 {
            fed_kll(&mut h, &frecord("b;x", 10_000.0));
        }
        let median = h.estimate_subpop_quantile(&[Some("a")], 0.5);
        assert!(
            (1.0..=101.0).contains(&median),
            "group `a` spans 1..=101, median estimated {median}"
        );
    }

    /// `k` is well past the group size here, so the cell retains everything and
    /// the answer is exact. Pinned because it is what makes a rank error at a
    /// larger dataset attributable to compaction and not to the grid.
    #[test]
    fn kll_is_exact_below_k() {
        let mut h = built_kll();
        for v in 1..=101 {
            fed_kll(&mut h, &frecord("a;x", v as f64));
        }
        assert_eq!(h.estimate_subpop_quantile(&[Some("a")], 0.0), 1.0);
        assert_eq!(h.estimate_subpop_quantile(&[Some("a")], 1.0), 101.0);
    }

    /// The library allocates a cell's retained slots once, so the footprint is
    /// analytic. This pins the reproduction, not the library — the value below is
    /// hand-derived from the decay series, not read back from the function.
    #[test]
    fn kll_cell_capacity_matches_the_library_shape() {
        // ceil(200 * (2/3)^i) for i in 0..8 is 200, 134, 89, 60, 40, 27, 18, 12
        // summing to 580; every level from 8 up is floored at m = 8, and there
        // are 53 of them, adding 424.
        assert_eq!(kll_cell_slots(200), 580 + 424);
        // Monotone in k, and never below the floor times the level count.
        assert!(kll_cell_slots(400) > kll_cell_slots(200));
        assert_eq!(kll_cell_slots(1), KLL_LIB_MIN_LEVEL * KLL_LIB_MAX_LEVELS);
        // Past the library's clamp, a larger k buys no more slots.
        assert_eq!(
            kll_cell_slots(KLL_LIB_MAX_CACHEABLE_K as u32),
            kll_cell_slots(KLL_LIB_MAX_CACHEABLE_K as u32 + 5_000)
        );
    }

    /// A cell is a whole `KLL`, merge buffer included, as the `kll-*` lib rows
    /// count it. Hand-derived: at `k = 200`, 1004 retained slots (above) plus
    /// 200 buffer items, 8 bytes each, plus 62 level offsets.
    #[test]
    fn kll_footprint_counts_each_cells_merge_buffer() {
        let per_cell = (1004 + 200) * 8 + 62 * std::mem::size_of::<usize>();
        assert_eq!(
            memory_hydra_kll(&built_kll()),
            3 * 64 * per_cell + grid_overhead_bytes(3, 64)
        );
    }

    /// Below `k` the cell keeps every value, so `Cdf(x)` is the exact share at
    /// or below `x`, inside the group only.
    #[test]
    fn kll_cdf_is_the_share_at_or_below_inside_the_group() {
        let mut h = built_kll();
        for v in 1..=100 {
            fed_kll(&mut h, &frecord("a;x", v as f64));
        }
        for _ in 0..300 {
            fed_kll(&mut h, &frecord("b;x", 0.5));
        }
        assert_eq!(h.estimate_subpop_cdf(&[Some("a")], 25.0), 0.25);
        assert_eq!(h.estimate_subpop_cdf(&[Some("a")], 100.0), 1.0);
        assert_eq!(h.estimate_subpop_cdf(&[Some("a")], 0.9), 0.0);
    }

    #[test]
    fn kll_zero_cell_k_is_refused_by_name() {
        let bad = ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 0,
        });
        let Err(err) = build_hydra_kll(&bad, 2) else {
            panic!("a zero cell_k must be refused, not built");
        };
        assert!(
            err.to_string().contains("cell_k"),
            "error should name the field: {err}"
        );
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::polars::*;
    use crate::params::{ParamSet, SketchParams};
    use crate::wrappers::hydra_kll::HydraKllParams;
    use std::rc::Rc;

    /// `a` holds 10, 10, 20: `F_a(10)` counts both tens, `F_a(15)` too, and
    /// `F_a(5)` nothing. A group never seen answers NaN, not a share.
    #[test]
    fn the_cdf_baseline_answers_the_share_at_or_below() {
        let stream = Rc::new(vec![
            ("a;x".to_string(), 10.0),
            ("a;y".to_string(), 10.0),
            ("a;x".to_string(), 20.0),
            ("b;x".to_string(), 30.0),
        ]);
        let a = || vec![Some("a".to_string())];
        let probes = Rc::new(vec![(a(), 10.0), (a(), 15.0), (a(), 5.0), (a(), 20.0)]);
        let params = ParamSet::of(&HydraKllParams::canonical());
        let mut passes =
            query_polars_subpop_cdf::<f64>(&params, stream, probes, 1).expect("baseline builds");
        let answers = passes.pop().expect("one pass")().0;
        assert_eq!(answers, vec![2.0 / 3.0, 2.0 / 3.0, 0.0, 1.0]);
    }
}
