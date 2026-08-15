//! `InitSketch` — build a sketch from a run's `ParamSet`, or say why not.
//!
//! The seam an implementation plugs into: construct yourself from the shared
//! init config and you are a benchmarkable row. There is no separate "which
//! configs does this impl accept" table — the answer is whatever `init` does.

use crate::config::{ParamSet, SketchParams};
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

/// A catalog row's identity — what the record is labelled with. Separate from
/// construction because every row has a name whether or not it builds the same
/// way.
///
/// Two names, on two axes. [`Self::IMPL`] says **who** implements it, which is
/// the library: `oxide`, `datasketches`, `lib`, `polars`. [`Self::ALGORITHM`]
/// says **what** is being implemented, down to the structural variant: `hll`
/// and `hll-hip` are two algorithms because a different estimator gives a
/// different answer, and `cms-fastpath-vector2d` and `cms-regularpath-vector2d`
/// are two because a different hash strategy does.
///
/// Variants of one structure share a parameter vocabulary, so they share a
/// [`Self::Params`], and `Params::FAMILY` is what groups them back together for
/// a cross-library comparison.
pub trait BenchImpl {
    /// The parameters this impl is built from. `Params::FAMILY` is the row's
    /// family.
    type Params: SketchParams;

    /// The implementing library (`"oxide"`, `"datasketches"`, `"lib"`,
    /// `"polars"`). Nothing else belongs here: a storage backend or a code path
    /// is part of *what* is being measured and goes in [`Self::ALGORITHM`].
    const IMPL: &'static str;

    /// The algorithm this row measures. Defaults to the family's base name, so
    /// a row with no structural variant writes nothing; a variant overrides it
    /// with `family-variant`. A `const` rather than a method so a catalog can
    /// read a row's identity off the type in `const` context, and build its
    /// list and its dispatch from one table.
    const ALGORITHM: &'static str = <Self::Params as SketchParams>::FAMILY;

    /// The family, derived — never written by hand. Rows sharing it answer the
    /// same question from the same knobs, which is what makes them comparable.
    const FAMILY: &'static str = <Self::Params as SketchParams>::FAMILY;

    // ---------- which operations this impl actually has ----------
    //
    // A row supplies `merge` and `prepare` as `Option<fn>` in its `SketchOps`,
    // so absence is honest at run time. But an `Option` inside a thunk is not
    // readable in `const` context, so a registry could not refuse
    // `--operations prepare` before generating a workload for it.
    //
    // These two say it out loud. Both default to `false`, matching the two
    // trait defaults, so an impl that overrides neither writes nothing and an
    // impl that overrides one says so on the line beside its `IMPL`.

    /// Does this row's [`SketchOps`](crate::ops::SketchOps) supply a `merge`?
    /// Must agree with it — `catalog`'s `declared_support_tests` pins that.
    const SUPPORTS_MERGE: bool = false;

    /// Does it supply a `prepare`? The doc's `prepare_for_query` axis: KLL's
    /// `cdf` rows build their lookup there, most sketches have nothing to do.
    const SUPPORTS_PREPARE: bool = false;
}
