//! Thin newtypes over each concrete sketch implementation in the
//! repo, one file per algorithm. Each file owns its sketches *and how they are
//! driven*: `InitSketch` builds one from a `ParamSet`, and a `SketchOps` names
//! the functions that insert into it, fold it, finalise it and ask it. Those
//! functions are written here, in the sketch's own terms — nothing forces two
//! files to agree on a signature.
//!
//! `registry::REGISTRY` names one `run_*` per row and nothing else.

// One directory per algorithm; inside each, one file per library. A reader
// looking for "the datasketches Count-Min" goes to `cms/datasketches.rs`, and
// finds the type, how it is built, how it is fed, and how it is asked, all in
// one place.
pub mod cms;
pub mod cs;
pub mod hll;
pub mod hydra;
pub mod kll;

// Shared by rows across several algorithms.
pub mod fixed_matrix;
pub mod polars_shared;

use crate::build_error::BuildError;
use asap_sketchlib::impl_fixed_matrix;

//
// What the three parallel-insert rows share. Their per-worker sketch is a
// compile-time type, so the shape is not a knob: a request naming any other one
// is refused rather than accepted and ignored. Each row itself lives in its
// algorithm's `sketchlib.rs`.
//
// Its own type, not the `M5x32768` in `fixed_matrix`: that table is the set of
// shapes the *sweepable* fixedmatrix rows dispatch over, and pruning it must not
// silently move the shape these rows are named after.

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// The shape every worker's matrix is baked at.
pub const PARALLEL_ROWS: usize = 5;

pub const PARALLEL_COLS: usize = 32768;

/// Contiguous ranges, one per worker. `n.max(1)` because a run of no threads is
/// not a run, and the empty-workload case still hands back one part rather than
/// none.
pub fn partition(items: &[i64], n: usize) -> Vec<&[i64]> {
    let n = n.max(1);
    let chunk = (items.len() + n - 1) / n;
    if chunk == 0 {
        return vec![items];
    }
    items.chunks(chunk).collect()
}

//
// All four helpers below exist for one rule: a row that cannot build at the
// requested parameters says so, and never builds at different ones under the
// requested label. The record's `sketch_config` is then always the config that
// ran, which is what lets a plot key on it.
//
// They divide by how the wrapped library states its domain. `require_shape`:
// one shape, baked into a type. `require_range` / `require_positive`: a bound
// the library asserts on, so it has to be checked before the call.
// `require_resolved_shape`: no stated domain at all, only what the library
// resolved the request to, which is readable off the built structure.

/// A wrapper whose `(rows, cols)` are baked into its type runs at exactly one
/// shape; any other requested config is a `BuildError` naming both shapes.
/// Shared by the fixed-matrix CMS and CountSketch rows, which read the shape
/// off the storage they were monomorphised at, and by the parallel rows, whose
/// per-worker sketch is a compile-time type.
pub(crate) fn require_shape(
    rows: usize,
    cols: usize,
    want_rows: usize,
    want_cols: usize,
) -> Result<(), BuildError> {
    if (rows, cols) != (want_rows, want_cols) {
        return Err(BuildError(format!(
            "fixed at {want_rows}x{want_cols}, requested {rows}x{cols}"
        )));
    }
    Ok(())
}

/// Refuse a parameter outside the range the wrapped library accepts, naming the
/// value and the bound.
///
/// The bound arrives as an argument because it is the *library's*, not the
/// parameter vocabulary's: `lg_k` is 4..=18 for one library and {12,14,16} for
/// another, and each row states its own. Use this wherever the library would
/// otherwise assert, panic or truncate.
pub(crate) fn require_range<T>(
    what: &str,
    name: &str,
    got: T,
    lo: T,
    hi: T,
) -> Result<(), BuildError>
where
    T: PartialOrd + std::fmt::Display,
{
    if got < lo || got > hi {
        return Err(BuildError(format!(
            "{what}: {name}={got} outside [{lo}, {hi}], which is what this library accepts"
        )));
    }
    Ok(())
}

/// Refuse a dimension of zero, for a library that has a floor and no ceiling.
///
/// Separate from [`require_range`] because printing `usize::MAX` as the upper
/// bound would state a limit the library does not have, and a reader chasing a
/// refusal should not have to work out that 18446744073709551615 means "no
/// ceiling".
pub(crate) fn require_positive(what: &str, name: &str, got: usize) -> Result<(), BuildError> {
    if got == 0 {
        return Err(BuildError(format!(
            "{what}: {name} must be at least 1, got {got}"
        )));
    }
    Ok(())
}

/// Refuse a `(rows, cols)` the library did not resolve to exactly, naming what
/// it built instead.
///
/// The `sketch_oxide` constructors take error bounds and derive the dimensions
/// back out of them, so a request survives only if it round-trips. Checking the
/// built sketch rather than replicating the library's rounding is what makes
/// this hold across a library upgrade: a changed formula turns into a refusal
/// here instead of a silently different table.
pub(crate) fn require_resolved_shape(
    what: &str,
    got: (usize, usize),
    want: (usize, usize),
) -> Result<(), BuildError> {
    if got != want {
        return Err(BuildError(format!(
            "{what} resolves rows={} cols={} to a {}x{} table; it derives its \
             dimensions from error bounds and rounds the width up to a power of \
             two, so ask for a power-of-two `cols`",
            want.0, want.1, got.0, got.1
        )));
    }
    Ok(())
}
