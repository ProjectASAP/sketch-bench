//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{check_grid, label_columns, labels, new_hydra, query, update};

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
                seed: None,
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
        // Ties count as at or below: every one of b's values is 0.5.
        assert_eq!(h.estimate_subpop_cdf(&[Some("b")], 0.5), 1.0);

        // Mixed ties, on a grid wide enough that `c` shares no cell: its 2s
        // are three of its five values.
        let mut wide = build_hydra_kll(
            &ParamSet::of(&HydraKllParams {
                rows: 3,
                cols: 4096,
                cell_k: 200,
                seed: None,
            }),
            2,
        )
        .expect("canonical dimensions build");
        for v in [1.0, 2.0, 2.0, 2.0, 3.0] {
            fed_kll(&mut wide, &frecord("c;y", v));
        }
        assert_eq!(wide.estimate_subpop_cdf(&[Some("c")], 2.0), 0.8);
        assert_eq!(wide.estimate_subpop_cdf(&[Some("c")], 1.0), 0.2);
    }

    #[test]
    fn kll_zero_cell_k_is_refused_by_name() {
        let bad = ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 0,
            seed: None,
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
