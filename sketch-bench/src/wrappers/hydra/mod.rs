//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;
use aqpbm_core::RunError;
use asap_sketchlib::input::HydraCounter;

pub mod polars;
pub mod sketchlib;

/// outer grid is the one shape they have in common.
fn check_grid(rows: usize, cols: usize, algorithm: &str) -> Result<(), RunError> {
    for (name, v) in [("rows", rows), ("cols", cols)] {
        if v == 0 {
            return Err(RunError::Target(format!("{algorithm}: {name} must be > 0")));
        }
    }
    Ok(())
}

/// Bytes the grid itself costs, on top of the counters inside the cells: every
/// cell is an enum around a sketch struct, plus the prototype `Hydra` clones
/// from. Separate from the counter bytes, so every row states both components.
fn grid_overhead_bytes(rows: usize, cols: usize) -> usize {
    (rows * cols + 1) * std::mem::size_of::<HydraCounter>()
}

/// Registers in one `HyperLogLog<ErtlMLE>` cell. The library fixes the cell at
/// `HyperLogLogP14`, so this is `2^14` and not a parameter. Named here because
/// the footprint depends on it and nothing in the params struct states it.
const HLL_CELL_REGISTERS: usize = 1 << 14;

// Level count and decay of an `asap_sketchlib::KLL`. Both are private constants
// in the library, reproduced here because a cell's footprint is a function of
// them and the library exposes no accessor for its own capacity.
const KLL_MAX_LEVELS: usize = 61;

const KLL_CAPACITY_DECAY: f64 = 2.0 / 3.0;

/// The `m` the library's `init_kll` passes, its minimum level capacity, and the
/// floor it silently raises a smaller `k` to. Same cell as the `kll` rows, so
/// the bound is theirs: see [`crate::wrappers::kll::LIB_K_MIN`].
const KLL_MIN_LEVEL: usize = crate::wrappers::kll::LIB_K_MIN as usize;

/// The library clamps `k` to this before sizing, so a larger `k` buys nothing.
const KLL_MAX_CACHEABLE_K: usize = crate::wrappers::kll::LIB_K_MAX as usize;

/// Retained slots one KLL cell allocates at construction — `KLL::init` boxes a
/// slice of this length once and never grows it. A line-for-line copy of the
/// library's private `compute_max_capacity`, so a claim about 0.2.2, not a bound.
fn kll_cell_slots(k: u32) -> usize {
    // `init_internal` normalises before sizing: `m` floors `k`, and `k` is
    // capped. Reproduced so an out-of-range `k` reports the footprint the
    // library actually allocates.
    let m = KLL_MIN_LEVEL;
    let k = (k as usize).max(m).min(KLL_MAX_CACHEABLE_K) as f64;
    let mut total = 0usize;
    let mut scale = 1.0f64;
    for _ in 0..KLL_MAX_LEVELS {
        total += (k * scale).ceil().max(m as f64) as usize;
        scale *= KLL_CAPACITY_DECAY;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use crate::params::SketchParams;
    use aqpbm_core::input_dataset::Labeled;

    fn built() -> HydraCms {
        build_hydra_cms(
            &ParamSet::of(&HydraCmsParams {
                rows: 3,
                cols: 64,
                cell_rows: 3,
                cell_cols: 256,
            }),
            1,
        )
        .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> Labeled<i64> {
        Labeled {
            key: key.to_string(),
            value,
        }
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
            insert_hydra_cms(&mut h, &r);
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
        insert_hydra_cms(&mut h, &record("a;x", 10));
        assert_eq!(h.estimate_subpop_frequency(&["zzz"], &10), 0.0);
    }

    /// Count-Min is linear and the grid is cell-wise, so folding two shards is
    /// exact, not approximate.
    #[test]
    fn merging_shards_is_exact() {
        let (mut left, mut right) = (built(), built());
        for _ in 0..3 {
            insert_hydra_cms(&mut left, &record("a;x", 10));
        }
        for _ in 0..4 {
            insert_hydra_cms(&mut right, &record("a;x", 10));
        }
        merge_hydra_cms(&mut left, &right);
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
        let Err(err) = build_hydra_cms(&bad, 1) else {
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
        assert!(build_hydra_cms(&ParamSet::of(&HydraCmsParams::canonical()), 1).is_ok());
        assert!(build_hydra_hll(&ParamSet::of(&HydraHllParams::canonical()), 1).is_ok());
        assert!(build_hydra_kll(&ParamSet::of(&HydraKllParams::canonical()), 1).is_ok());
    }

    // ---------- hydra-hll ----------

    fn built_hll() -> HydraHll {
        build_hydra_hll(&ParamSet::of(&HydraHllParams { rows: 3, cols: 64 }), 1)
            .expect("canonical dimensions build")
    }

    /// The statistic is the *size* of the group, so repeats of one value must
    /// not raise it. This is the property that separates this row from the
    /// Count-Min one, which would answer 3 here.
    #[test]
    fn hll_counts_distinct_values_not_occurrences() {
        let mut h = built_hll();
        for _ in 0..3 {
            insert_hydra_hll(&mut h, &record("a;x", 10));
        }
        insert_hydra_hll(&mut h, &record("a;y", 20));
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
        insert_hydra_hll(&mut h, &record("a;x", 10));
        assert_eq!(h.estimate_subpop_cardinality(&["zzz"]), 0.0);
    }

    /// A HyperLogLog is register-wise mergeable, so two shards fold without
    /// double counting the value they share.
    #[test]
    fn hll_merging_shards_does_not_double_count() {
        let (mut left, mut right) = (built_hll(), built_hll());
        insert_hydra_hll(&mut left, &record("a;x", 10));
        insert_hydra_hll(&mut left, &record("a;x", 20));
        insert_hydra_hll(&mut right, &record("a;x", 20));
        insert_hydra_hll(&mut right, &record("a;x", 30));
        merge_hydra_hll(&mut left, &right);
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

    // ---------- hydra-kll ----------

    fn built_kll() -> HydraKll {
        build_hydra_kll(
            &ParamSet::of(&HydraKllParams {
                rows: 3,
                cols: 64,
                cell_k: 200,
            }),
            1,
        )
        .expect("canonical dimensions build")
    }

    fn frecord(key: &str, value: f64) -> Labeled<f64> {
        Labeled {
            key: key.to_string(),
            value,
        }
    }

    /// The statistic is ordered and taken inside a group, so the median of one
    /// group must not be pulled by another group's values.
    #[test]
    fn kll_quantiles_are_taken_inside_the_group() {
        let mut h = built_kll();
        for v in 1..=101 {
            insert_hydra_kll(&mut h, &frecord("a;x", v as f64));
        }
        for _ in 0..500 {
            insert_hydra_kll(&mut h, &frecord("b;x", 10_000.0));
        }
        let median = h.estimate_subpop_quantile(&["a"], 0.5);
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
            insert_hydra_kll(&mut h, &frecord("a;x", v as f64));
        }
        assert_eq!(h.estimate_subpop_quantile(&["a"], 0.0), 1.0);
        assert_eq!(h.estimate_subpop_quantile(&["a"], 1.0), 101.0);
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
        assert_eq!(kll_cell_slots(1), KLL_MIN_LEVEL * KLL_MAX_LEVELS);
        // Past the library's clamp, a larger k buys no more slots.
        assert_eq!(
            kll_cell_slots(KLL_MAX_CACHEABLE_K as u32),
            kll_cell_slots(KLL_MAX_CACHEABLE_K as u32 + 5_000)
        );
    }

    #[test]
    fn kll_zero_cell_k_is_refused_by_name() {
        let bad = ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 0,
        });
        let Err(err) = build_hydra_kll(&bad, 1) else {
            panic!("a zero cell_k must be refused, not built");
        };
        assert!(
            err.to_string().contains("cell_k"),
            "error should name the field: {err}"
        );
    }
}
