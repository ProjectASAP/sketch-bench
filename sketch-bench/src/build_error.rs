//! Why a sketch could not be built from a given `ParamSet`.
//!
//! The type lives in `aqpbm-core`, because the `build` closure that crate is
//! handed returns one; re-exported here because every message one carries is
//! written in this crate.

pub use aqpbm_core::build_error::BuildError;
