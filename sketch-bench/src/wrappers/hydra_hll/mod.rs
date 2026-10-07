//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{
    check_grid, grid_overhead_bytes, labels, merge, new_hydra, query, update,
};

/// Registers in one `HyperLogLog<ErtlMLE>` cell. The library fixes the cell at
/// `HyperLogLogP14`, so this is `2^14` and not a parameter. Named here because
/// the footprint depends on it and nothing in the params struct states it.
const HLL_CELL_REGISTERS: usize = 1 << 14;

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;

    fn built_hll() -> HydraHll {
        build_hydra_hll(&ParamSet::of(&HydraHllParams { rows: 3, cols: 64 }))
            .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> (String, i64) {
        (key.to_string(), value)
    }

    fn fed_hll(sketch: &mut HydraHll, r: &(String, i64)) {
        update(
            &mut sketch.inner,
            &r.0,
            &asap_sketchlib::DataInput::I64(r.1),
        );
    }

    #[test]
    fn canonical_params_build() {
        assert!(build_hydra_hll(&ParamSet::of(&HydraHllParams::canonical())).is_ok());
    }

    /// The statistic is the *size* of the group, so repeats of one value must
    /// not raise it. This is the property that separates this row from the
    /// Count-Min one, which would answer 3 here.
    #[test]
    fn hll_counts_distinct_values_not_occurrences() {
        let mut h = built_hll();
        for _ in 0..3 {
            fed_hll(&mut h, &record("a;x", 10));
        }
        fed_hll(&mut h, &record("a;y", 20));
        // Two distinct values under label `a`, seen four times.
        let est = h.estimate_subpop_cardinality(&["a"]);
        assert!(
            (est - 2.0).abs() < 0.5,
            "two distinct values under `a`, estimated {est}"
        );
    }

    #[test]
    fn hll_absent_group_estimates_zero() {
        let mut h = built_hll();
        fed_hll(&mut h, &record("a;x", 10));
        assert_eq!(h.estimate_subpop_cardinality(&["zzz"]), 0.0);
    }

    /// A HyperLogLog is register-wise mergeable, so two shards fold without
    /// double counting the value they share.
    #[test]
    fn hll_merging_shards_does_not_double_count() {
        let (mut left, mut right) = (built_hll(), built_hll());
        fed_hll(&mut left, &record("a;x", 10));
        fed_hll(&mut left, &record("a;x", 20));
        fed_hll(&mut right, &record("a;x", 20));
        fed_hll(&mut right, &record("a;x", 30));
        merge(&mut left.inner, &right.inner);
        let est = left.estimate_subpop_cardinality(&["a"]);
        assert!(
            (est - 3.0).abs() < 0.5,
            "three distinct values across both shards, estimated {est}"
        );
    }

    /// The cell is fixed-shape, so the footprint is the grid area times a
    /// constant, and no construction parameter can move it.
    #[test]
    fn hll_footprint_is_grid_area_times_a_fixed_cell() {
        let h = built_hll();
        assert_eq!(
            memory_hydra_hll(&h),
            3 * 64 * HLL_CELL_REGISTERS + grid_overhead_bytes(3, 64)
        );
    }

    #[test]
    fn zero_dimensions_are_refused_by_name() {
        let bad = ParamSet::of(&HydraHllParams { rows: 3, cols: 0 });
        let Err(err) = build_hydra_hll(&bad) else {
            panic!("a zero dimension must be refused, not built");
        };
        let err = err.to_string();
        assert!(
            err.contains("hydra-hll"),
            "error should name the row: {err}"
        );
        assert!(err.contains("cols"), "error should name the field: {err}");
    }
}
