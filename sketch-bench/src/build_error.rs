//! Why a sketch could not be built from a given `ParamSet`.
//!
//! The type itself lives in `aqpbm-core`, because the `build` closure that
//! crate is handed returns one. Re-exported here so a wrapper file reaches it by
//! the path it has always used: every message a `BuildError` carries is written
//! in this crate.

pub use aqpbm_core::build_error::BuildError;
