//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{
    check_grid, grid_overhead_bytes, label_columns, labels, new_hydra, query, update,
};

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;

    fn built() -> HydraUnivmon {
        build_hydra_univmon(
            &ParamSet::of(&HydraUnivmonParams {
                rows: 2,
                cols: 16,
                cell_heap_size: 32,
                cell_sketch_row: 5,
                cell_sketch_col: 256,
                cell_layer_size: 4,
            }),
            2,
        )
        .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> (String, i64) {
        (key.to_string(), value)
    }

    fn fed(sketch: &mut HydraUnivmon, r: &(String, i64)) {
        update(
            &mut sketch.inner,
            &r.0,
            &asap_sketchlib::DataInput::I64(r.1),
            "hydra-univmon",
        );
    }

    #[test]
    fn canonical_params_build() {
        assert!(build_hydra_univmon(&ParamSet::of(&HydraUnivmonParams::canonical()), 2).is_ok());
    }

    #[test]
    fn l1_of_a_group_is_how_many_records_it_carried() {
        let mut h = built();
        for _ in 0..3 {
            fed(&mut h, &record("a;x", 10));
        }
        fed(&mut h, &record("a;y", 20));
        let est = h.estimate_subpop_l1_norm(&[Some("a")]);
        assert!(
            (est - 4.0).abs() < 0.5,
            "four records under `a`, estimated {est}"
        );
    }

    #[test]
    fn cardinality_counts_distinct_values_not_occurrences() {
        let mut h = built();
        for _ in 0..3 {
            fed(&mut h, &record("a;x", 10));
        }
        fed(&mut h, &record("a;y", 20));
        let est = h.estimate_subpop_cardinality(&[Some("a")]);
        assert!(
            (est - 2.0).abs() < 0.5,
            "two distinct values under `a`, estimated {est}"
        );
    }

    #[test]
    fn entropy_is_zero_when_the_group_held_one_value() {
        let mut h = built();
        for _ in 0..8 {
            fed(&mut h, &record("a;x", 10));
        }
        let est = h.estimate_subpop_entropy(&[Some("a")]);
        assert!(
            est.abs() < 0.1,
            "one value, so no uncertainty, estimated {est}"
        );
    }

    #[test]
    fn l2_of_a_single_valued_group_is_its_l1() {
        let mut h = built();
        for _ in 0..4 {
            fed(&mut h, &record("a;x", 10));
        }
        let l1 = h.estimate_subpop_l1_norm(&[Some("a")]);
        let l2 = h.estimate_subpop_l2_norm(&[Some("a")]);
        assert!((l1 - l2).abs() < 0.5, "l1 {l1}, l2 {l2}");
    }

    #[test]
    fn merging_shards_sums_the_group() {
        let (mut left, mut right) = (built(), built());
        for _ in 0..3 {
            fed(&mut left, &record("a;x", 10));
        }
        for _ in 0..4 {
            fed(&mut right, &record("a;x", 10));
        }
        left.inner
            .merge(&right.inner)
            .expect("both operands built from one ParamSet, so shapes match");
        let est = left.estimate_subpop_l1_norm(&[Some("a")]);
        assert!(
            (est - 7.0).abs() < 0.5,
            "seven records in all, estimated {est}"
        );
    }

    /// Each record counted by its own value makes the group's L1 its sum. At
    /// a grid far wider than the four subkeys, every cell holds one group.
    #[test]
    fn weighted_l1_of_a_group_is_its_sum() {
        let params = ParamSet::of(&HydraUnivmonParams {
            rows: 3,
            cols: 1024,
            cell_heap_size: 32,
            cell_sketch_row: 5,
            cell_sketch_col: 256,
            cell_layer_size: 4,
        });
        let stream = std::rc::Rc::new(vec![
            record("a;x", 10),
            record("a;y", 10),
            record("a;x", 25),
            record("b;x", 7),
        ]);
        let probes = std::rc::Rc::new(vec![
            vec![Some("a".to_string())],
            vec![Some("b".to_string())],
            vec![None, Some("x".to_string())],
        ]);
        let mut passes = query_hydra_univmon_sum(&params, stream, probes, 1).expect("builds");
        let answers = passes.pop().expect("one pass")().0;
        assert_eq!(answers, vec![45.0, 7.0, 42.0]);
    }

    /// The library's weight is an `i32` count: a value it cannot carry is
    /// refused by name, not truncated into a different sum.
    #[test]
    fn a_value_past_an_i32_count_is_refused_by_name() {
        let params = ParamSet::of(&HydraUnivmonParams::canonical());
        for bad in [i64::from(i32::MAX) + 1, -1] {
            let stream = std::rc::Rc::new(vec![record("a;x", 1), record("a;x", bad)]);
            let Err(err) = insert_hydra_univmon_sum(&params, stream, 1) else {
                panic!("{bad} must be refused, not inserted");
            };
            let err = err.to_string();
            assert!(
                err.contains("hydra-univmon-sum") && err.contains(&bad.to_string()),
                "{err}"
            );
        }
    }

    #[test]
    fn zero_dimensions_are_refused_by_name() {
        let bad = ParamSet::of(&HydraUnivmonParams {
            rows: 2,
            cols: 16,
            cell_heap_size: 32,
            cell_sketch_row: 5,
            cell_sketch_col: 0,
            cell_layer_size: 4,
        });
        let Err(err) = build_hydra_univmon(&bad, 2) else {
            panic!("a zero dimension must be refused, not built");
        };
        let err = err.to_string();
        assert!(
            err.contains("cell_sketch_col"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn footprint_is_the_grid_area_times_one_univmon() {
        let h = built();
        let cell =
            4 * ((5 * 256 + 5) * 8 + 32 * std::mem::size_of::<asap_sketchlib::input::HHItem>());
        assert_eq!(
            memory_hydra_univmon(&h),
            2 * 16 * cell + grid_overhead_bytes(2, 16)
        );
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::polars::*;
    use crate::params::{ParamSet, SketchParams};
    use crate::wrappers::hydra_univmon::HydraUnivmonParams;
    use std::rc::Rc;

    fn stream() -> Rc<Vec<(String, i64)>> {
        Rc::new(vec![
            ("a;x".to_string(), 10),
            ("a;y".to_string(), 10),
            ("a;x".to_string(), 20),
            ("b;x".to_string(), 30),
        ])
    }

    type Asked = fn(
        &ParamSet,
        Rc<Vec<(String, i64)>>,
        Rc<Vec<Vec<Option<String>>>>,
        usize,
    )
        -> Result<Vec<crate::wrappers::QueryPass<f64>>, crate::wrappers::BuildError>;

    fn answers_for(query: Asked) -> Vec<f64> {
        let params = ParamSet::of(&HydraUnivmonParams::canonical());
        let probes = Rc::new(vec![
            vec![Some("a".to_string())],
            vec![Some("b".to_string())],
        ]);
        let mut passes = query(&params, stream(), probes, 1).expect("baseline builds");
        let pass = passes.pop().expect("one pass");
        pass().0
    }

    #[test]
    fn the_l1_baseline_answers_the_group_size() {
        assert_eq!(
            answers_for(query_polars_subpop_l1_norm::<i64>),
            vec![3.0, 1.0]
        );
    }

    /// `a` holds 10, 10, 20; its L1 sibling answers 3 for it.
    #[test]
    fn the_sum_baseline_answers_the_group_sum() {
        assert_eq!(
            answers_for(query_polars_subpop_sum::<i64>),
            vec![40.0, 30.0]
        );
    }

    #[test]
    fn the_cardinality_baseline_answers_distinct_values() {
        assert_eq!(
            answers_for(query_polars_subpop_cardinality_univmon::<i64>),
            vec![2.0, 1.0]
        );
    }
}
