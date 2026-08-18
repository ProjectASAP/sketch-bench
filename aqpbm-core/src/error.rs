//! Why a measurement could not be produced. Worded elsewhere — core carries it.

/// Why a measurement could not be produced.
///
/// Only two ways, both of which happen before anything is timed: the data could
/// not be produced, or the target said it could not run.
#[derive(Debug)]
pub enum RunError {
    InputDataSet(anyhow::Error),
    /// The target reported it could not run — a construction config it cannot
    /// satisfy, a fold with nothing to fold. The registry words it; core
    /// carries it.
    Target(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::InputDataSet(e) => e.fmt(f),
            RunError::Target(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for RunError {}

/// So that `spec.build::<T>()?` inside `crate::target` converts without a map.
impl From<anyhow::Error> for RunError {
    fn from(e: anyhow::Error) -> Self {
        RunError::InputDataSet(e)
    }
}

/// A config that does not parse into an impl's params type is the target
/// refusing to be built, reported before a dataset is generated.
impl From<crate::DataGenError> for RunError {
    fn from(e: crate::DataGenError) -> Self {
        RunError::Target(e.to_string())
    }
}
