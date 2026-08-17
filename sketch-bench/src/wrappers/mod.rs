//! Thin newtypes over each sketch, one directory per algorithm and one file per
//! library, each owning its `build_*` / `insert_*` / `ask_*` / `memory_*`. Not a
//! trait, so no two files must agree on a signature; they capture nothing.

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

// The library types a caller has to *name* to select one of the monomorphisations
// below. Re-exported so picking a shape or a precision does not mean depending on
// `asap_sketchlib` directly — this crate is where the wrapped libraries live.
pub use asap_sketchlib::{
    DefaultXxHasher, FastPathHasher, HllBucketListP12, HllBucketListP14, HllBucketListP16,
    MatrixStorage,
};

// What the three parallel-insert rows share: a per-worker sketch whose shape is
// a compile-time type, so any other request is refused. Its own type, not the
// `M5x32768` in `fixed_matrix`, so pruning that table cannot move this shape.

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// The shape every worker's matrix is baked at.
pub const PARALLEL_ROWS: usize = 5;

pub const PARALLEL_COLS: usize = 32768;

/// Contiguous ranges, one per worker. `n.max(1)` because a run of no threads is
/// not a run, and the empty-dataset case still hands back one part rather than
/// none.
pub fn partition(items: &[i64], n: usize) -> Vec<&[i64]> {
    let n = n.max(1);
    let chunk = items.len().div_ceil(n);
    if chunk == 0 {
        return vec![items];
    }
    items.chunks(chunk).collect()
}

// All four helpers below exist for one rule: a row that cannot build at the
// requested parameters says so, and never builds at different ones under the
// requested label — so `sketch_config` is always the config that ran.

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
/// value and the bound. The bound is an argument because it is the *library's* —
/// `lg_k` is 4..=18 for one and {12,14,16} for another, so each row states its own.
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

/// Refuse a dimension of zero, for a library with a floor and no ceiling.
/// Separate from [`require_range`] because printing `usize::MAX` as the upper
/// bound would state a limit the library does not have.
pub(crate) fn require_positive(what: &str, name: &str, got: usize) -> Result<(), BuildError> {
    if got == 0 {
        return Err(BuildError(format!(
            "{what}: {name} must be at least 1, got {got}"
        )));
    }
    Ok(())
}

/// Refuse a `(rows, cols)` the library did not resolve to exactly, naming what it
/// built instead. Checking the built sketch rather than replicating the rounding
/// is what holds across an upgrade: a changed formula refuses here.
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
