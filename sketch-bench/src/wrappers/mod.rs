//! Thin newtypes over each concrete sketch implementation in the
//! repo, one file per algorithm. Each implements `Accumulator` (drive it)
//! and `InitSketch` (build it from a `ParamSet`); `sketch_bench::catalog`
//! binds the runtime `(algorithm, impl)` strings to these types.

pub mod cms;
pub mod countsketch;
pub mod dd;
pub mod elastic;
pub mod fixed_matrix;
pub mod hll;
pub mod hydra;
pub mod kll;
pub mod nitro;
pub mod parallel;
pub mod polars;
pub mod topk;
pub mod univmon;

use aqpbm_core::init::BuildError;

// ---------- refusing a config the row cannot honour ----------
//
// Both helpers below exist for one rule: a row that cannot build at the
// requested parameters says so, and never builds at different ones under the
// requested label. The record's `sketch_config` is then always the config that
// ran, which is what lets a plot key on it.

/// A wrapper whose `(rows, cols)` are baked into its type runs at exactly one
/// shape; any other requested config is a `BuildError` naming both shapes.
/// Shared by the fixed CMS and CountSketch wrappers.
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
