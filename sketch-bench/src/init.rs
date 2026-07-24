//! `InitSketch` — build a sketch from a run's `ParamSet`, or say why not.
//!
//! This is the seam an implementation plugs into. Give a way to construct
//! yourself from the shared init config and you become a benchmarkable row;
//! nothing else about you needs to be registered.
//!
//! ## Construction is where a config is accepted or rejected
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
//! across libraries so the numbers mean something. What each impl does with
//! it is private to that impl.
//!
//! ## Scope
//!
//! `init` takes only the `ParamSet`. An impl needing a *run* knob that is not
//! a sketch parameter — the parallel-insert wrappers want the worker count
//! from `--workers`, which lives in `BenchConfig`, not here — cannot be
//! expressed through this trait and keeps its own constructor.

use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;
use aqpbm_core::SketchError;

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
/// `Self: Sketch` because the only thing worth building here is something the
/// runner can drive.
pub trait InitSketch: Sketch + Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}
