//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{
    check_grid, grid_overhead_bytes, labels, merge, new_hydra, query, update,
};

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;

    fn built() -> HydraUnivmon {
        build_hydra_univmon(&ParamSet::of(&HydraUnivmonParams {
            rows: 2,
            cols: 16,
            cell_heap_size: 32,
            cell_sketch_row: 5,
            cell_sketch_col: 256,
            cell_layer_size: 4,
        }))
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
        );
    }

    #[test]
    fn canonical_params_build() {
        assert!(build_hydra_univmon(&ParamSet::of(&HydraUnivmonParams::canonical())).is_ok());
    }

    #[test]
    fn l1_of_a_group_is_how_many_records_it_carried() {
        let mut h = built();
        for _ in 0..3 {
            fed(&mut h, &record("a;x", 10));
        }
        fed(&mut h, &record("a;y", 20));
        let est = h.estimate_subpop_l1_norm(&["a"]);
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
        let est = h.estimate_subpop_cardinality(&["a"]);
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
        let est = h.estimate_subpop_entropy(&["a"]);
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
        let l1 = h.estimate_subpop_l1_norm(&["a"]);
        let l2 = h.estimate_subpop_l2_norm(&["a"]);
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
        merge(&mut left.inner, &right.inner);
        let est = left.estimate_subpop_l1_norm(&["a"]);
        assert!(
            (est - 7.0).abs() < 0.5,
            "seven records in all, estimated {est}"
        );
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
        let Err(err) = build_hydra_univmon(&bad) else {
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
        Rc<Vec<Vec<String>>>,
        usize,
    )
        -> Result<Vec<crate::wrappers::QueryPass<f64>>, crate::wrappers::BuildError>;

    fn answers_for(query: Asked) -> Vec<f64> {
        let params = ParamSet::of(&HydraUnivmonParams::canonical());
        let probes = Rc::new(vec![vec!["a".to_string()], vec!["b".to_string()]]);
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

    #[test]
    fn the_cardinality_baseline_answers_distinct_values() {
        assert_eq!(
            answers_for(query_polars_subpop_cardinality_univmon::<i64>),
            vec![2.0, 1.0]
        );
    }
}
