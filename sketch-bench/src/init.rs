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
/// Carried back to the sweep so a skipped `(impl, config)` cell can say what
/// was wrong: a config that did not parse, or a request an impl's fixed shape
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

/// Reason the parameter-free baselines (exact / null / polars) currently
/// refuse to build. See `docs/component_walk_through.md`.
///
/// These baselines have an *empty* parameter space: they produce the exact
/// reference answer regardless of any sketch's `(rows, cols)`. But the sweep
/// today enumerates one grid per family and forces every impl onto it, so a
/// baseline was handed a `CmsParams` it could only discard — "succeeding" by
/// ignoring the config. That is a category error the old `unparameterized`
/// flag papered over by un-sweeping these rows after the fact.
///
/// Rejecting construction here makes it honest: the benchmark runs without
/// exact/null/polars baselines until they are modelled as a single empty
/// grid point rather than special-cased off the family grid. Temporary.
pub const BASELINE_NO_PARAM_SPACE: &str = "baseline has an empty parameter \
    space and does not belong on the family's (rows, cols) grid; temporarily \
    disabled pending per-impl grid enumeration";

/// Build `Self` from the shared init config, or explain why not.
///
/// `Self: Sketch` because the only thing worth building here is something the
/// runner can drive.
pub trait InitSketch: Sketch + Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}
