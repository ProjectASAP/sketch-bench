//! Thin newtypes over each sketch, one directory per algorithm and one file per
//! library, each owning its `build_*` / `insert_*` / `query_*` / `memory_*`. Not a
//! trait, so no two files must agree on a signature; they capture nothing.

// One directory per algorithm; inside each, one file per library. A reader
// looking for "the datasketches Count-Min" goes to `cms/datasketches.rs`, and
// finds the type, how it is built, how it is fed, and how it is asked, all in
// one place.
pub mod cms;
pub mod cms_heap;
pub mod cs;
pub mod dd;
pub mod hll;
pub mod hydra_cms;
pub mod hydra_hll;
pub mod hydra_kll;
pub mod kll;
pub mod univmon;

// Shared by rows across several algorithms.
pub mod fixed_matrix;
pub mod frequency_value;
pub mod hydra_shared;
pub mod polars_shared;
pub mod quantile_value;

use crate::params::ParamSet;
use aqpbm_datagen::DataGenError;
use asap_sketchlib::impl_fixed_matrix;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Why a row could not be built at the requested config. The wrappers' own
/// error: nothing here knows what a benchmark is, so nothing here reports in
/// the framework's vocabulary.
#[derive(Debug)]
pub struct BuildError(pub String);

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

impl From<DataGenError> for BuildError {
    fn from(e: DataGenError) -> Self {
        BuildError(e.to_string())
    }
}

/// One primed run of an operation: a sketch is already built, and calling this
/// does the work. It hands back the footprint of the sketch it drops on the way
/// out, which is the one thing a caller cannot read once the sketch is gone.
pub type Pass = Box<dyn FnOnce() -> usize>;

/// The same, for an operation that answers: the answers in probe order, plus
/// the footprint. Scoring them is the caller's business, not the sketch's.
pub type QueryPass<A> = Box<dyn FnOnce() -> (Vec<A>, usize)>;

/// The same work, one unit at a time, for a caller that times each call. The
/// units are the row's own — items for an insert, shards for a fold — and the
/// stream they come from stays inside the closure.
pub struct StepPass {
    /// How many calls make one whole pass.
    pub steps: usize,
    /// One unit of work; `i` counts from zero.
    pub step: Box<dyn FnMut(usize)>,
    /// Read once the calls are done.
    pub footprint: Box<dyn Fn() -> usize>,
}

/// A sketch shared by the two halves of a [`StepPass`]: driven by `step`, read
/// by `footprint`. Both hold it, so it is counted; each call borrows for the
/// length of one unit of work.
pub type Shared<S> = Rc<RefCell<S>>;

/// Prime `passes` runs of an insert: the stream is fed to a sketch of its own
/// each time.
pub type InsertBody<I> = fn(&ParamSet, Rc<Vec<I>>, usize) -> Result<Vec<Pass>, BuildError>;

/// The same insert, driven per item.
pub type InsertStepBody<I> = fn(&ParamSet, Rc<Vec<I>>, usize) -> Result<Vec<StepPass>, BuildError>;

/// The same for a row whose ingest is one call over the whole stream: it reads
/// the worker count, which no other row does.
pub type ParallelInsertBody<I> =
    fn(&ParamSet, usize, Rc<Vec<I>>, usize) -> Result<Vec<Pass>, BuildError>;

/// Prime `passes` runs of a query: each sketch is built and fed here, so the
/// pass asks and only asks.
pub type QueryBody<I, P, A> =
    fn(&ParamSet, Rc<Vec<I>>, Rc<Vec<P>>, usize) -> Result<Vec<QueryPass<A>>, BuildError>;

/// Prime `passes` runs of a fold over `shards` sketches, each already fed.
pub type MergeBody<I> = fn(&ParamSet, Rc<Vec<I>>, usize, usize) -> Result<Vec<Pass>, BuildError>;

/// The same fold, driven one shard at a time.
pub type MergeStepBody<I> =
    fn(&ParamSet, Rc<Vec<I>>, usize, usize) -> Result<Vec<StepPass>, BuildError>;

/// Both forms of a row's fold: as one pass, and one shard at a time. A row
/// either supplies both or has no merge at all.
pub type Folds<I> = (MergeBody<I>, MergeStepBody<I>);

/// Prime `passes` runs of the step that makes a fed sketch ready to answer.
pub type PrepareBody<I> = fn(&ParamSet, Rc<Vec<I>>, usize) -> Result<Vec<Pass>, BuildError>;

// The library types a caller has to *name* to select one of the monomorphisations
// below. Re-exported so picking a shape or a precision does not mean depending on
// `asap_sketchlib` directly — this crate is where the wrapped libraries live.
pub use asap_sketchlib::{
    DefaultXxHasher, FastPathHasher, HllBucketListP12, HllBucketListP14, HllBucketListP16,
    MatrixStorage,
};

// What the three parallel-insert rows share: a per-worker sketch whose shape is
// a compile-time type, so any other request is refused. Its own type, not the
// `M5x32768` in `fixed_matrix`, so pruning that table cannot move this shape.

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// The shape every worker's matrix is baked at.
pub const PARALLEL_ROWS: usize = 5;

pub const PARALLEL_COLS: usize = 32768;

/// Contiguous ranges, one per worker. `n.max(1)` because a run of no threads is
/// not a run, and the empty-dataset case still hands back one part rather than
/// none.
pub fn partition<T>(items: &[T], n: usize) -> Vec<&[T]> {
    let n = n.max(1);
    let chunk = items.len().div_ceil(n);
    if chunk == 0 {
        return vec![items];
    }
    items.chunks(chunk).collect()
}

// All four helpers below exist for one rule: a row that cannot build at the
// requested parameters says so, and never builds at different ones under the
// requested label — so `sketch_config` is always the config that ran.

/// A wrapper whose `(rows, cols)` are baked into its type runs at exactly one
/// shape; any other requested config is a `BuildError` naming both shapes.
/// Shared by the fixed-matrix CMS and CountSketch rows, which read the shape
/// off the storage they were monomorphised at, and by the parallel rows, whose
/// per-worker sketch is a compile-time type.
pub(crate) fn require_shape(
    rows: usize,
    cols: usize,
    want_rows: usize,
    want_cols: usize,
) -> Result<(), BuildError> {
    if (rows, cols) != (want_rows, want_cols) {
        return Err(BuildError(format!(
            "fixed at {want_rows}x{want_cols}, requested {rows}x{cols}"
        )));
    }
    Ok(())
}

/// Refuse a parameter outside the range the wrapped library accepts, naming the
/// value and the bound. The bound is an argument because it is the *library's* —
/// `lg_k` is 4..=18 for one and {12,14,16} for another, so each row states its own.
pub(crate) fn require_range<T>(
    what: &str,
    name: &str,
    got: T,
    lo: T,
    hi: T,
) -> Result<(), BuildError>
where
    T: PartialOrd + std::fmt::Display,
{
    if got < lo || got > hi {
        return Err(BuildError(format!(
            "{what}: {name}={got} outside [{lo}, {hi}], which is what this library accepts"
        )));
    }
    Ok(())
}

/// Refuse a dimension of zero, for a library with a floor and no ceiling.
/// Separate from [`require_range`] because printing `usize::MAX` as the upper
/// bound would state a limit the library does not have.
pub(crate) fn require_positive(what: &str, name: &str, got: usize) -> Result<(), BuildError> {
    if got == 0 {
        return Err(BuildError(format!(
            "{what}: {name} must be at least 1, got {got}"
        )));
    }
    Ok(())
}

/// Refuse a `(rows, cols)` the library did not resolve to exactly, naming what it
/// built instead. Checking the built sketch rather than replicating the rounding
/// is what holds across an upgrade: a changed formula refuses here.
pub(crate) fn require_resolved_shape(
    what: &str,
    got: (usize, usize),
    want: (usize, usize),
) -> Result<(), BuildError> {
    if got != want {
        return Err(BuildError(format!(
            "{what} resolves rows={} cols={} to a {}x{} table; it derives its \
             dimensions from error bounds and rounds the width up to a power of \
             two, so ask for a power-of-two `cols`",
            want.0, want.1, got.0, got.1
        )));
    }
    Ok(())
}
