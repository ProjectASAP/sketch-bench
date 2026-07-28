//! Thin newtypes over each concrete sketch implementation in the
//! repo, one file per family. Each implements `Accumulator` (drive it)
//! and `InitSketch` (build it from a `ParamSet`); `sketch_bench::catalog`
//! binds the runtime `(family, impl)` strings to these types.

pub mod cms;
pub mod countsketch;
pub mod dd;
pub mod elastic;
pub mod hll;
pub mod hydra;
pub mod kll;
pub mod nitro;
pub mod parallel;
pub mod polars;
pub mod topk;
pub mod univmon;

use aqpbm_core::init::BuildError;

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
