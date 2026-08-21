//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{check_grid, grid_overhead_bytes, labels};

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;

    fn built() -> HydraCms {
        build_hydra_cms(&ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: 64,
            cell_rows: 3,
            cell_cols: 256,
        }))
        .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> (String, i64) {
        (key.to_string(), value)
    }

    fn fed(sketch: &mut HydraCms, r: &(String, i64)) {
        sketch
            .inner
            .update(&r.0, &asap_sketchlib::DataInput::I64(r.1), None);
    }

    /// The statistic is the frequency of a value *within* a subpopulation, and
    /// the coarse query must see every record whose first label matches, not
    /// only those whose full key does.
    #[test]
    fn a_coarse_query_covers_every_record_in_the_group() {
        let mut h = built();
        for r in [
            record("a;x", 10),
            record("a;y", 10),
            record("a;x", 20),
            record("b;x", 30),
        ] {
            fed(&mut h, &r);
        }
        // (a, 10) occurs twice, under two different second labels.
        assert_eq!(h.estimate_subpop_frequency(&["a"], &10), 2.0);
        assert_eq!(h.estimate_subpop_frequency(&["a"], &20), 1.0);
        assert_eq!(h.estimate_subpop_frequency(&["b"], &30), 1.0);
        // The full key is its own subpopulation and is stored too.
        assert_eq!(h.estimate_subpop_frequency(&["a", "x"], &10), 1.0);
    }

    /// A group that never occurred must estimate zero. This is the failure mode
    /// a grouped sketch has and an ungrouped one does not, so it is worth
    /// pinning even on a grid large enough to make collisions unlikely.
    #[test]
    fn an_absent_group_estimates_zero() {
        let mut h = built();
        fed(&mut h, &record("a;x", 10));
        assert_eq!(h.estimate_subpop_frequency(&["zzz"], &10), 0.0);
    }

    /// Count-Min is linear and the grid is cell-wise, so folding two shards is
    /// exact, not approximate.
    #[test]
    fn merging_shards_is_exact() {
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
        assert_eq!(left.estimate_subpop_frequency(&["a"], &10), 7.0);
    }

    #[test]
    fn zero_dimensions_are_refused_by_name() {
        let bad = ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: 0,
            cell_rows: 3,
            cell_cols: 256,
        });
        let Err(err) = build_hydra_cms(&bad) else {
            panic!("a zero dimension must be refused, not built");
        };
        let err = err.to_string();
        assert!(err.contains("cols"), "error should name the field: {err}");
    }

    /// Footprint is the product of both shapes plus the grid's own cells, which
    /// is the property that makes the two dimension pairs non-interchangeable.
    #[test]
    fn footprint_is_the_product_of_both_shapes() {
        let h = built();
        let counters = 3 * 64 * 3 * 256 * 4;
        assert_eq!(memory_hydra_cms(&h), counters + grid_overhead_bytes(3, 64));
        // The counters still dominate, so the overhead term must not be what
        // the number is mostly made of.
        assert!(memory_hydra_cms(&h) < counters * 2);
    }

    #[test]
    fn canonical_params_build() {
        assert!(build_hydra_cms(&ParamSet::of(&HydraCmsParams::canonical())).is_ok());
    }
}
