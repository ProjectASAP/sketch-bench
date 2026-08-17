//! KLL Wrapper
//! specialty: KLL may (very likely) to have a `prepare()` phase
//! which is a phase to do some precomputation
//! and support multiple quantile queries from the precomputation
//! not from KLL itself

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// figure is `heap_bytes_net`, which the tracking allocator measures; this is
/// the derived upper bound beside it, and the two are meant to be compared.
/// One rule for all four rows, so the column answers one question.
fn kll_footprint<T>(k: u32) -> usize {
    (k as usize) * std::mem::size_of::<T>() * 4
}

/// The range `asap_sketchlib::KLL::init` keeps a `k` in. Below the floor it
/// raises `k` to `m`, above the ceiling it caps; both silently. Reproduced here
/// so the two `lib` rows refuse instead, which is the only way the `k` in the
/// record is the `k` that ran. See [`LIB_K_RANGE`]'s use in `hydra.rs`, which
/// has the same cell and the same bound.
pub const LIB_K_MIN: u32 = 8;

pub const LIB_K_MAX: u32 = 26_602;

/// Answer a quantile out of a prebuilt `(value, cumulative_rank)` table, the
/// shape `sketch_oxide::KllSketch::cdf` returns: ascending by value, with the
/// cumulative rank normalised to `[0, 1]`.
///
/// `min` / `max` are passed in and short-circuit the ends, because the per-call
/// path special-cases them too and a gratuitous divergence at `phi = 0` and
/// `phi = 1` would show up as rank error that belongs to neither path.
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
