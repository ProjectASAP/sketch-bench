//! `InitSketch` — build a sketch from a run's `ParamSet`, or say why not.
//!
//! The seam an implementation plugs into: construct yourself from the shared
//! init config and you are a benchmarkable row. There is no separate "which
//! configs does this impl accept" table — the answer is whatever `init` does.

use crate::config::ParamSet;
use crate::DataGenError;

/// Why a sketch could not be built from a given `ParamSet`: a config that did
/// not parse, or a request an impl's fixed shape cannot satisfy. Surfaced as
/// the invocation's error, so a cell that cannot run says what was wrong.
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

/// Build `Self` from the shared init config, or explain why not. `Self:
/// Accumulator` because the only thing worth building is one the runner drives.
pub trait InitSketch: Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}
