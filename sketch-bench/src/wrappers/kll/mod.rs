//! KLL Wrapper
//! specialty: KLL may (very likely) to have a `prepare()` phase
//! which is a phase to do some precomputation
//! and support multiple quantile queries from the precomputation
//! not from KLL itself

/// grounds, which rank-error analysis does not care about.
pub trait ToF64 {
    fn to_f64(self) -> f64;
}

impl ToF64 for f64 {
    fn to_f64(self) -> f64 {
        self
    }
}
impl ToF64 for f32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}

/// A value an ordered (quantile) sketch can ingest. Adds a **total** order
/// over [`ToF64`]: `f64` is only partially ordered, so `partial_cmp().unwrap()`
/// panics on NaN. Use `f64::total_cmp`; integers just use `Ord::cmp`.
pub trait QuantileValue: ToF64 + Copy {
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering;
}

impl QuantileValue for i64 {
    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(self, other)
    }
}

impl QuantileValue for f64 {
    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        f64::total_cmp(self, other)
    }
}

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
