//! Thin newtypes over each concrete sketch implementation in the
//! repo, one file per family. Each implements `Sketch` (drive it)
//! and `InitSketch` (build it from a `ParamSet`); `dispatch.rs`
//! picks between them at CLI parse time.

pub mod cms;
pub mod countsketch;
pub mod dd;
pub mod elastic;
pub mod hll;
pub mod kll;
pub mod nitro;
pub mod parallel;
pub mod polars;
pub mod univmon;

use crate::init::BuildError;

/// A wrapper whose `(rows, cols)` are baked into its type runs at exactly one
/// grid point; every other point of a sweep is a `BuildError` naming both
/// shapes. Shared by the fixed CMS and CountSketch wrappers.
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
