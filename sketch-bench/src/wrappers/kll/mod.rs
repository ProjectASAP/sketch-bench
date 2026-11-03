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
/// report. The `lib` rows report the library's own `memory_bytes` instead.
fn kll_footprint<T>(k: u32) -> usize {
    (k as usize) * std::mem::size_of::<T>() * 4
}

/// The `m` the library's `with_k` passes (`DEFAULT_M`), its minimum level
/// capacity, and the floor it silently raises a smaller `k` to.
pub(crate) const KLL_LIB_MIN_LEVEL: usize = 8;

// The lib rows refuse a `k` below the library's floor rather than let it be
// raised silently, so the two must agree.
const _: () = assert!(LIB_K_MIN as usize == KLL_LIB_MIN_LEVEL);

/// The range `asap_sketchlib::Kll::new` keeps a `k` in. Below the floor it
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
