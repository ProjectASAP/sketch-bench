//! Why a measurement could not be produced.
//!
//! Sibling of [`crate::build_error`], and the same division: `BuildError` is one
//! sketch refusing one `ParamSet`, this is a whole square that never became a
//! measurement. Both are worded elsewhere — core carries them.

/// Why a measurement could not be produced.
///
/// Only two ways, both of which happen before anything is timed: the data could
/// not be produced, or the body said it could not run.
#[derive(Debug)]
pub enum RunError {
    Dataset(anyhow::Error),
    /// The body reported it could not run — a build the config cannot satisfy,
    /// a fold with nothing to fold. The registry words it; core carries it.
    Body(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Dataset(e) => e.fmt(f),
            RunError::Body(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for RunError {}

/// So that `spec.build::<T>()?` inside `crate::ops` converts without a map.
impl From<anyhow::Error> for RunError {
    fn from(e: anyhow::Error) -> Self {
        RunError::Dataset(e)
    }
}
