//! Why a sketch could not be built from a given `ParamSet`.
//!
//! Lives here because the `build` closure [`crate::ops::squares_for`] takes
//! returns one: core never constructs a sketch, but it calls the thing that
//! does, so it has to be able to name the failure. What the message *says* is
//! still the wrapper's — every `BuildError` in this workspace is worded in
//! `sketch-bench/src/wrappers/`.

use crate::DataGenError;

/// A config that did not parse, or a request an implementation's fixed shape
/// cannot satisfy. Reported by the wrapper before a workload is generated.
#[derive(Debug, Clone)]
pub struct BuildError(pub String);

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

impl From<DataGenError> for BuildError {
    /// A `ParamSet` that does not parse into an impl's params type is a
    /// construction failure, reported through `config.parse::<P>()?`.
    fn from(e: DataGenError) -> Self {
        BuildError(e.to_string())
    }
}
