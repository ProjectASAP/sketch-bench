//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::params::*;

pub mod polars;
pub mod sketchlib;

pub(crate) use super::hydra_shared::{
    check_grid, grid_overhead_bytes, label_columns, labels, new_hydra, query, update,
};

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
            fed_kll(&mut h, &frecord("a;x", v as f64));
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
        let Err(err) = build_hydra_kll(&bad, 2) else {
            panic!("a zero cell_k must be refused, not built");
        };
        assert!(
            err.to_string().contains("cell_k"),
            "error should name the field: {err}"
        );
    }
}
