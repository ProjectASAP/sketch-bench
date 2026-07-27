//! `InitSketch` — build a sketch from a run's `ParamSet`, or say why not.
//!
//! The seam an implementation plugs into: construct yourself from the shared
//! init config and you are a benchmarkable row. There is no separate "which
//! configs does this impl accept" table — the answer is whatever `init` does.

use crate::accumulator::Accumulator;
use crate::config::{ParamSet, SketchParams};
use crate::SketchError;

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

impl From<SketchError> for BuildError {
    /// A `ParamSet` that does not parse into an impl's params type is a
    /// construction failure, reported through `config.parse::<P>()?`.
    fn from(e: SketchError) -> Self {
        BuildError(e.to_string())
    }
}

/// Build `Self` from the shared init config, or explain why not. `Self:
/// Accumulator` because the only thing worth building is one the runner drives.
pub trait InitSketch: Accumulator + Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}

/// A catalog row's identity — what the record is labelled with. Separate from
/// construction because every row has a name whether or not it builds the same
/// way. The family is read off `Params::FAMILY`, never spelled here.
pub trait BenchImpl: Accumulator {
    /// The parameters this impl is built from. `Params::FAMILY` is the row's
    /// family.
    type Params: SketchParams;

    /// This impl's name within the family (`"oxide"`, `"lib-hip"`, ...).
    const IMPL: &'static str;

    /// The family, derived — never written by hand. A `const` rather than a
    /// method so a catalog can read a row's identity off the type in `const`
    /// context, and build its list and its dispatch from one table.
    const FAMILY: &'static str = <Self::Params as SketchParams>::FAMILY;
}
