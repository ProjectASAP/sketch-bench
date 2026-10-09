//! KLL Wrapper
//! specialty: KLL may (very likely) to have a `prepare()` phase
//! which is a phase to do some precomputation
//! and support multiple quantile queries from the precomputation
//! not from KLL itself

pub use super::quantile_value::{QuantileValue, ToF64};

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// A nominal footprint for the `oxide` rows, `4 · k` items: `sketch_oxide`'s
/// KLL grows its buffers as it fills, so there is no fixed allocation to
/// report. The `lib` rows report the allocation instead ([`kll_lib_bytes`]).
fn kll_footprint<T>(k: u32) -> usize {
    (k as usize) * std::mem::size_of::<T>() * 4
}

// Level count and decay of an `asap_sketchlib::KLL`. Both are private constants
// in the library, reproduced here because its footprint is a function of them
// and the library exposes no accessor for its own capacity.
pub(crate) const KLL_LIB_MAX_LEVELS: usize = 61;

const KLL_LIB_CAPACITY_DECAY: f64 = 2.0 / 3.0;

/// The `m` the library's `init_kll` passes, its minimum level capacity, and the
/// floor it silently raises a smaller `k` to.
pub(crate) const KLL_LIB_MIN_LEVEL: usize = LIB_K_MIN as usize;

/// The library clamps `k` to this before sizing, so a larger `k` buys nothing.
pub(crate) const KLL_LIB_MAX_CACHEABLE_K: usize = LIB_K_MAX as usize;

/// The `k` the library sizes with: `init_internal` floors it at `m` and caps it.
fn kll_lib_k(k: u32) -> usize {
    (k as usize).clamp(KLL_LIB_MIN_LEVEL, KLL_LIB_MAX_CACHEABLE_K)
}

/// Retained slots an `asap_sketchlib::KLL` allocates at construction:
/// `KLL::init` boxes a slice of this length once and never grows it. A
/// line-for-line copy of the library's private `compute_max_capacity`
/// (unchanged from 0.2.2 to 0.3.0), so a claim about the pinned version, not a
/// bound.
pub(crate) fn kll_lib_slots(k: u32) -> usize {
    let m = KLL_LIB_MIN_LEVEL;
    let k = kll_lib_k(k) as f64;
    let mut total = 0usize;
    let mut scale = 1.0f64;
    for _ in 0..KLL_LIB_MAX_LEVELS {
        total += (k * scale).ceil().max(m as f64) as usize;
        scale *= KLL_LIB_CAPACITY_DECAY;
    }
    total
}

/// Bytes an `asap_sketchlib::KLL<T>` allocates at construction, whatever it
/// holds later (`init_internal`): the retained-item slice, the level index,
/// and the merge buffer's capacity of `k` items. Independent of the item count,
/// so a window's sketch costs the same however many values it sees.
fn kll_lib_bytes<T>(k: u32) -> usize {
    kll_lib_slots(k) * std::mem::size_of::<T>()
        + (KLL_LIB_MAX_LEVELS + 1) * std::mem::size_of::<usize>()
        + kll_lib_k(k) * std::mem::size_of::<T>()
}

/// The range `asap_sketchlib::KLL::init` keeps a `k` in. Below the floor it
/// raises `k` to `m`, above the ceiling it caps; both silently. Reproduced here
/// so the two `lib` rows refuse instead, which is the only way the `k` in the
/// record is the `k` that ran. The `hydra-kll` row bounds its `cell_k` the same
/// way, against the same library and the same limits.
pub const LIB_K_MIN: u32 = 8;

pub const LIB_K_MAX: u32 = 26_602;

/// Answer a quantile out of a prebuilt `(value, cumulative_rank)` table, the
/// shape `sketch_oxide::KllSketch::cdf` returns. `min` / `max` short-circuit the
/// ends, as the per-call path does, so `phi = 0` and `1` do not diverge.
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
    use super::*;

    /// The `lib` footprint is what `init_internal` allocates. At k = 269 that
    /// is 9648 + 496 + 2152 = 12296 B, the figure `aqpbm-planeval` derives for
    /// the same library independently.
    #[test]
    fn lib_footprint_is_the_library_allocation() {
        assert_eq!(kll_lib_bytes::<f64>(269), 9648 + 496 + 2152);
        // ceil(50 · (2/3)^i) is 50, 34, 23, 15, 10, then 56 levels at m = 8.
        assert_eq!(kll_lib_slots(50), 132 + 56 * 8);
        // Below the floor, the library sizes as k = m.
        assert_eq!(kll_lib_bytes::<i64>(1), kll_lib_bytes::<i64>(LIB_K_MIN));
    }
}
