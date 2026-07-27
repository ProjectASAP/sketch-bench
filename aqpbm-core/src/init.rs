//! `InitSketch` — build a sketch from a run's `ParamSet`, or say why not.
//!
//! This is the seam an implementation plugs into. Give a way to construct
//! yourself from the shared init config and you become a benchmarkable row;
//! nothing else about you needs to be registered.
//!
//! There is deliberately no separate "which configs does this impl accept"
//! table. The answer is whatever `init` does with the `ParamSet`:
//!
//! - a tunable impl reads its knobs and builds;
//! - an impl with a compile-time-fixed shape checks the request against that
//!   shape and returns [`BuildError`] when they disagree;
//! - an impl whose native API speaks a different language (sketch_oxide's CMS
//!   takes an error bound, not `rows`/`cols`) translates inside `init`.
//!
//! The `ParamSet` is the shared axis of comparison — the knobs held equal
//! across libraries so the numbers mean something.
//!
//! `init` takes only the `ParamSet`, so an impl needing a *run* knob — the
//! parallel-insert wrappers want the worker count from `--workers`, which
//! lives in `BenchConfig` — cannot be expressed through this trait and keeps
//! its own constructor.

use crate::accumulator::Accumulator;
use crate::config::{ParamSet, SketchParams};
use crate::SketchError;

/// Why a sketch could not be built from a given `ParamSet`.
///
/// Surfaced as the invocation's error so a cell that cannot run says what was
/// wrong: a config that did not parse, or a request an impl's fixed shape
/// cannot satisfy. Plays the same role for the construction axis that
/// `DtypeMismatch` plays for the data-type axis — a reason, not just a "no".
#[derive(Debug, Clone)]
pub struct BuildError(pub String);

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

impl From<SketchError> for BuildError {
    /// A `ParamSet` that does not parse into an impl's params type — a
    /// misspelled key, a wrong-family set — is a construction failure, so
    /// `config.parse::<P>()?` inside `init` reports through this.
    fn from(e: SketchError) -> Self {
        BuildError(e.to_string())
    }
}

/// Build `Self` from the shared init config, or explain why not.
///
/// `Self: Accumulator` because the only thing worth building here is something the
/// runner can drive.
pub trait InitSketch: Accumulator + Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}

/// A catalog row's identity — what the record is labelled with.
///
/// Separate from construction (`InitSketch` / `cell::ParallelInit`, which the
/// parallel rows implement instead) because every row has a name whether or
/// not it builds the same way.
///
/// The family is *not* spelled here: it is read off `Params::FAMILY`, so
/// `"cms"` is written once — in `CmsParams` — and the label on the emitted
/// record cannot drift from the one in the catalog
/// (`catalog::tests::every_catalog_entry_runs` pins the two together).
///
/// That is the whole guarantee. Nothing ties `Params` to the type
/// `InitSketch::init` actually parses — pointing a row's `Params` at another
/// family's struct compiles, and only fails at run time, when the row is
/// handed a config of the family it now claims and its `init` will not parse
/// it.
pub trait BenchImpl: Accumulator {
    /// The parameters this impl is built from. `Params::FAMILY` is the row's
    /// family.
    type Params: SketchParams;

    /// This impl's name within the family (`"oxide"`, `"lib-hip"`, ...).
    const IMPL: &'static str;

    /// The family, derived — never written by hand.
    ///
    /// A `const` rather than a method so a catalog can read a row's identity
    /// off the type in a `const` context, and so build its list of rows and
    /// its dispatch from one table instead of two.
    const FAMILY: &'static str = <Self::Params as SketchParams>::FAMILY;
}
