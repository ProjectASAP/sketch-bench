//! Why a sketch could not be built from a given `ParamSet`.
//!
//! Lived in `aqpbm-core` while core did the building. It does not any more —
//! a body constructs its own sketch before core ever sees it — so the error
//! belongs to the crate that constructs.

use aqpbm_core::DataGenError;

/// A config that did not parse, or a request an implementation's fixed shape
/// cannot satisfy. Reported by the registry before a workload is generated.
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
