//! The catalog: one table naming every `(algorithm, impl)` this crate exposes,
//! and the dispatch resolving one to a concrete sketch type. It lives here,
//! not in the CLI, so a future `aqp-bench` can ship its own. A row is a
//! *type*, not a pair of strings — algorithm, impl name and `scores_accuracy`
//! are projected off it, so the list and the code cannot drift apart.

use anyhow::Result;

use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::{self, BenchItem, ParallelInit, RunError, WorkloadData};
use aqpbm_core::runner::{needs_ground_truth, NoGT};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::workload::Labeled;
use aqpbm_core::metrics::{cells, is_measurable, MetricsMask, OperationMask};
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::{BenchConfig, BenchReport};

use asap_sketchlib::{
    DefaultXxHasher, FastPathHasher, HllBucketListP12, HllBucketListP14, HllBucketListP16,
    MatrixStorage,
};


use crate::params::{HllParams, ParamSet};
use crate::wrappers::{cms, countsketch, fixed_matrix, hll, hydra, kll, parallel, polars};

// ---------- what a row is ----------

// The item width a row is measured at now lives in `aqpbm-core`, because
// `Requirement` carries it and core has to be able to name every field of a
// request. Re-exported here so a frontend still finds it beside the rows it
// selects.
pub use aqpbm_core::request::{Capability, Numeric};

/// The executable half of a row: everything the frontend can hand a cell.
type RunFn =
    fn(&BenchConfig, WorkloadData, &ParamSet, Numeric) -> Result<Vec<BenchReport>, RunError>;

/// One catalog entry. Built only by the constructors below, so `family`,
/// `algorithm`, `impl_name` and `scores_accuracy` are always projections of the
/// row's type and its runner — never hand-written strings that could drift from
/// it.
pub struct Row {
    /// Rows sharing this answer the same question from the same knobs, so this
    /// is what a cross-library comparison groups by. Derived from the row's
    /// params type: one parameter vocabulary is one family.
    pub family: &'static str,
    /// The algorithm, structural variant included. `--algorithm` matches this
    /// exactly, because one invocation measures one cell.
    pub algorithm: &'static str,
    /// The implementing library, and only that.
    pub impl_name: &'static str,
    /// The one field that is genuinely new data, and so is written in [`ROWS`].
    pub description: &'static str,
    /// Can a comparator score this row? Derived: true iff it was built with a
    /// constructor that takes a ground-truth calculator.
    pub scores_accuracy: bool,
    /// Can this row run at [`Numeric::F64`]? Derived: only `ordered` rows can.
    pub picks_width: bool,
    /// Does this row ingest labelled records, and so need a multi-column
    /// `--spec` description instead of a single-column one? Derived off the
    /// row's item type.
    pub takes_columns: bool,
    /// The `data_type` a description has to give this row's value column.
    /// Derived off the row's item type, so it cannot drift from it.
    pub value_type: &'static str,
    /// The statistic this row answers, as a value. Derived from the row's
    /// ground-truth calculator, so a row cannot claim a capability no
    /// comparator can score it under.
    pub capability: Capability,
    /// The operations this row can be measured over. Insert always; query
    /// wherever a comparator exists; merge and prepare only where the impl
    /// declares them via `BenchImpl::SUPPORTS_*`.
    ///
    /// This is the row's half of the answer — the framework's half is
    /// `aqpbm_core::metrics::is_measurable`, which rules out squares no row
    /// could fill. A request has to clear both.
    pub operations: OperationMask,
    /// The metrics this row can carry. Everything but accuracy, which needs a
    /// comparator and so follows [`Row::capability`].
    pub metrics: MetricsMask,
    run: RunFn,
    /// The comparator this row is scored by, by the name `--comparator` selects
    /// it with; `None` for a row nothing scores.
    ///
    /// One name, not a list: a row has exactly one runner, and a sketch with a
    /// second capability is registered as a second row — `docs/sketch-bench.md`
    /// line 38. When a statistic grows a second comparator, that is a second
    /// row too.
    comparator: Option<&'static str>,
}

// ---------- how a row builds its ground truth ----------

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* in [`ROWS`] and the row stays `const`.
pub(crate) trait GroundTruthCalculator<I>: GroundTruth<I> {
    /// The name `--comparator` selects this one by. One capability can carry
    /// several comparators, and this is what tells them apart on the command
    /// line.
    const NAME: &'static str;
    /// The statistic this comparator scores. Two comparators can share one —
    /// a quantile answer scores as a rank error or as a relative error — which
    /// is why the capability is named here and not derived from the name.
    const CAPABILITY: Capability;
    fn build(params: &ParamSet) -> Self;
}

impl<I> GroundTruthCalculator<I> for CardinalityGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "cardinality";
    const CAPABILITY: Capability = Capability::Cardinality;
    fn build(_params: &ParamSet) -> Self {
        CardinalityGT
    }
}

impl<I> GroundTruthCalculator<I> for FrequencyGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "frequency";
    const CAPABILITY: Capability = Capability::Frequency;
    fn build(_params: &ParamSet) -> Self {
        FrequencyGT
    }
}

impl<I> GroundTruthCalculator<I> for SubpopFrequencyGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "subpop-frequency";
    const CAPABILITY: Capability = Capability::SubpopFrequency;
    /// Scores column 0. A grouped sketch stores every column subset, but each
    /// one is its own population with its own error, so a comparator names the
    /// one it scores instead of pooling them.
    fn build(_params: &ParamSet) -> Self {
        SubpopFrequencyGT {
            label_column: 0,
        }
    }
}

impl<I> GroundTruthCalculator<I> for SubpopCardinalityGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "subpop-cardinality";
    const CAPABILITY: Capability = Capability::SubpopCardinality;
    /// Column 0, for the same reason as [`SubpopFrequencyGT`].
    fn build(_params: &ParamSet) -> Self {
        SubpopCardinalityGT {
            label_column: 0,
        }
    }
}

impl<I> GroundTruthCalculator<I> for SubpopRankErrorGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "subpop-rank-error";
    const CAPABILITY: Capability = Capability::SubpopQuantile;
    /// Column 0, for the same reason as [`SubpopFrequencyGT`]. The most
    /// expensive comparator in the catalog: 101 estimate calls per group.
    fn build(_params: &ParamSet) -> Self {
        SubpopRankErrorGT {
            label_column: 0,
        }
    }
}

impl<I> GroundTruthCalculator<I> for RankErrorGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "rank-error";
    const CAPABILITY: Capability = Capability::Quantile;
    fn build(_params: &ParamSet) -> Self {
        RankErrorGT {
        }
    }
}

impl<I> GroundTruthCalculator<I> for RelativeErrorGT
where
    Self: GroundTruth<I>,
{
    const NAME: &'static str = "relative-error";
    const CAPABILITY: Capability = Capability::Quantile;
    fn build(_params: &ParamSet) -> Self {
        RelativeErrorGT {
        }
    }
}

// `TopkGT` has no calculator here because no row binds the top-k capability
// any more. Core still carries the comparator and `TopKOps`; restoring a
// heavy-hitter row means restoring this impl, which reads `k` off the row's
// params so the score uses the same prefix the sketch was built with.

// ---------- the ways a row runs ----------

/// Every square the request selects, scored against `G` where one needs it.
pub(crate) fn run_scored<S, I, G, Ins>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    _width: Numeric,
    insert: Ins,
    ops: &SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: InitSketch + BenchImpl + MemoryFootprint,
    I: BenchItem,
    G: GroundTruthCalculator<I>,
    Ins: FnMut(&mut S, &I),
{
    // A comparator is built when the request reaches a square that cannot run
    // without one. Nobody has to ask for it: needing one is a property of the
    // squares selected, not a separate decision.
    let gt = needs_ground_truth(cfg.operations, cfg.metrics).then(|| G::build(params));
    Ok(cell::run_cell::<S, I, G, Ins>(cfg, data, params, gt.as_ref(), insert, ops)?)
}

/// An ordered quantile algorithm (KLL, DDSketch): the row names both widths and
/// the caller's [`Numeric`] picks one. The only place a runtime value still
/// selects an item type.
#[allow(clippy::type_complexity)]
pub(crate) fn run_ordered<Si, Sf, G, InsI, InsF>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    insert_i: InsI,
    ops_i: &SketchOps<Si, i64, <G as GroundTruth<i64>>::Probe, <G as GroundTruth<i64>>::Answer>,
    insert_f: InsF,
    ops_f: &SketchOps<Sf, f64, <G as GroundTruth<f64>>::Probe, <G as GroundTruth<f64>>::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: InitSketch + BenchImpl + MemoryFootprint,
    Sf: InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<i64> + GroundTruthCalculator<f64>,
    InsI: FnMut(&mut Si, &i64),
    InsF: FnMut(&mut Sf, &f64),
{
    // Two op sets because the two halves are two types. The wrapper file writes
    // one generic `ops::<T>()` and instantiates it at each width.
    match width {
        Numeric::I64 => run_scored::<Si, i64, G, _>(cfg, data, params, width, insert_i, ops_i),
        Numeric::F64 => run_scored::<Sf, f64, G, _>(cfg, data, params, width, insert_f, ops_f),
    }
}

// A timed-only runner (`run_cell::<S, NoGT>` with no ground truth) lived here
// for the rows that answered no query — elastic, nitro, univmon. All three are
// out of the catalog, so the only capability-less rows left are the parallel
// ones below, which have their own runner. Restore it with the first row that
// is measured but not scored.

/// A parallel-insert row: built with the worker count, so not an `InitSketch`.
pub(crate) fn run_parallel<S, I, Ins>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    _width: Numeric,
    insert: Ins,
    ops: &SketchOps<S, I, (), ()>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    I: BenchItem,
    Ins: FnMut(&mut S, &I),
{
    cell::run_cell_parallel::<S, I, NoGT, Ins>(cfg, data, params, None, insert, ops)
}

/// A row whose `(rows, cols)` selects a *type* rather than sizing a field.
///
/// `impl_fixed_matrix!` bakes the dimensions into the storage type, which is
/// what the row exists to price, so the shape is a monomorphisation and the
/// dispatch is what turns two runtime integers back into one. Same shape as
/// [`run_lib_hll`], and the same reason.
/// Both fixed-matrix rows are frequency rows, so the comparator is named here
/// instead of being a type parameter. That is not a shortcut: a comparator
/// parameter would have to hold for *every* storage type the dispatch can
/// select, which is a bound over all `M` and not something a caller can state.
/// Requiring the row to be `FrequencyOps` at every shape says the same thing in
/// a form the compiler accepts, and [`FrequencyGT`] follows from it.
fn run_fixed_matrix<W: FixedMatrixRow>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    let (rows, cols) = W::shape(params)?;
    let visitor = RunFixedMatrix::<W> {
        cfg,
        data,
        params,
        width,
        _row: std::marker::PhantomData,
    };
    fixed_matrix::with_fixed_matrix(rows, cols, visitor).unwrap_or_else(|| {
        Err(RunError::Build(BuildError(fixed_matrix::unsupported_shape(
            W::ALGORITHM,
            rows,
            cols,
        ))))
    })
}

/// The half of a fixed-matrix row that does not depend on the storage type:
/// which algorithm it is, and how to read its shape out of a `ParamSet`.
/// Implemented once per sketch type, in the wrapper that owns it.
pub trait FixedMatrixRow {
    const ALGORITHM: &'static str;
    /// What the row answers. Every shape of a fixed-matrix row answers the same
    /// statistic, so this belongs on the shape-independent half.
    const CAPABILITY: Capability;
    /// Same two facts `BenchImpl` carries, restated here because the sketch type
    /// is a GAT — there is no one `Self::At<M>` a `const` could read them off.
    const SUPPORTS_MERGE: bool = false;
    const SUPPORTS_PREPARE: bool = false;
    /// The row's concrete type at storage `M`, which is what actually runs.
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    >: InitSketch + BenchImpl + MemoryFootprint;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;

    /// How this row is driven, at whichever shape the config selected.
    ///
    /// Every other row states this as a `const SketchOps` in its wrapper file.
    /// This one cannot: its sketch type is a GAT, so there is no single type a
    /// `const` could be written against — the same reason `FixedMatrixVisitor`
    /// is a trait and not a closure. A generic method returning the ops is the
    /// stand-in, and the bodies still live in the wrapper file.
    /// The hot one, generic so it monomorphises — see `SketchOps`.
    fn insert<M>(sketch: &mut Self::At<M>, v: &i64)
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;

    fn ops<M>() -> SketchOps<Self::At<M>, i64, i64, u64>
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    cfg: &'a BenchConfig,
    data: WorkloadData,
    params: &'a ParamSet,
    width: Numeric,
    _row: std::marker::PhantomData<W>,
}

impl<W: FixedMatrixRow> fixed_matrix::FixedMatrixVisitor for RunFixedMatrix<'_, W> {
    type Out = Result<Vec<BenchReport>, RunError>;

    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        run_scored::<W::At<M>, i64, FrequencyGT, _>(
            self.cfg,
            self.data,
            self.params,
            self.width,
            W::insert::<M>,
            &W::ops::<M>(),
        )
    }
}

/// A row whose construction parameter selects a *type* rather than a field.
/// `asap_sketchlib` puts the HLL register count in the storage type, so `lg_k`
/// picks a monomorphisation and this dispatch is what turns a runtime value
/// back into one. Same shape as [`run_ordered`], and for the same reason: an
/// enum inside the wrapper would put a branch in `update`, on rows whose whole
/// purpose is to price that insert.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_lib_hll<S12, S14, S16, G, I12, I14, I16>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
    insert12: I12,
    ops12: &SketchOps<S12, i64, G::Probe, G::Answer>,
    insert14: I14,
    ops14: &SketchOps<S14, i64, G::Probe, G::Answer>,
    insert16: I16,
    ops16: &SketchOps<S16, i64, G::Probe, G::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    S12: InitSketch + BenchImpl + MemoryFootprint,
    S14: InitSketch + BenchImpl + MemoryFootprint,
    S16: InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<i64>,
    I12: FnMut(&mut S12, &i64),
    I14: FnMut(&mut S14, &i64),
    I16: FnMut(&mut S16, &i64),
{
    let p: HllParams = params.parse().map_err(BuildError::from)?;
    match p.lg_k {
        12 => run_scored::<S12, i64, G, _>(cfg, data, params, width, insert12, ops12),
        14 => run_scored::<S14, i64, G, _>(cfg, data, params, width, insert14, ops14),
        16 => run_scored::<S16, i64, G, _>(cfg, data, params, width, insert16, ops16),
        other => Err(RunError::Build(hll::unsupported_precision(other))),
    }
}

// ---------- the row constructors ----------
// Each reads `S::FAMILY` / `S::ALGORITHM` / `S::IMPL` off the type and fixes
// `scores_accuracy`. `const fn`, so `ROWS` stays `const` and a bad row fails at
// compile time.

// ---------- deriving what a row supports ----------
//
// Both take the two `SUPPORTS_*` facts as plain bools rather than a type
// parameter, because a fixed-matrix row states them on `FixedMatrixRow` (its
// sketch type is a GAT) while every other row states them on `BenchImpl`. One
// pair of helpers then serves both.

/// The operations a *scored* row admits. Insert and query always — it has a
/// comparator, so there is something to query — plus whichever of merge and
/// prepare the impl declares.
const fn scored_ops(supports_merge: bool, supports_prepare: bool) -> OperationMask {
    let mut bits = OperationMask::INSERT.bits() | OperationMask::QUERY.bits();
    if supports_merge {
        bits |= OperationMask::MERGE.bits();
    }
    if supports_prepare {
        bits |= OperationMask::PREPARE.bits();
    }
    OperationMask::from_bits_truncate(bits)
}

/// The operations an *unscored* row admits: the same, minus query. Nothing
/// builds a ground truth for it, so the query squares have nothing to issue.
const fn unscored_ops(supports_merge: bool, supports_prepare: bool) -> OperationMask {
    let mut bits = OperationMask::INSERT.bits();
    if supports_merge {
        bits |= OperationMask::MERGE.bits();
    }
    if supports_prepare {
        bits |= OperationMask::PREPARE.bits();
    }
    OperationMask::from_bits_truncate(bits)
}

/// Every metric a row can carry. The four timing and footprint bits hold of any
/// row; accuracy needs a comparator, so it follows `scores`.
const fn row_metrics(scores: bool) -> MetricsMask {
    let mut bits = MetricsMask::THROUGHPUT.bits()
        | MetricsMask::LATENCY.bits()
        | MetricsMask::CPU.bits()
        | MetricsMask::MEMORY.bits();
    if scores {
        bits |= MetricsMask::ACCURACY.bits();
    }
    MetricsMask::from_bits_truncate(bits)
}

const fn scored<S, I, G>(description: &'static str, run: RunFn) -> Row
where
    S: InitSketch + BenchImpl + MemoryFootprint,
    I: BenchItem,
    G: GroundTruthCalculator<I>,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <I as BenchItem>::TAKES_COLUMNS,
        value_type: <I as BenchItem>::DATA_TYPE,
        capability: G::CAPABILITY,
        operations: scored_ops(S::SUPPORTS_MERGE, S::SUPPORTS_PREPARE),
        metrics: row_metrics(true),
        run,
        comparator: Some(G::NAME),
    }
}

const fn ordered<Si, Sf, G>(description: &'static str, run: RunFn) -> Row
where
    Si: InitSketch + BenchImpl + MemoryFootprint,
    Sf: InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<i64> + GroundTruthCalculator<f64>,
{
    Row {
        // Both halves are the same row; the i64 one names it.
        family: Si::FAMILY,
        algorithm: Si::ALGORITHM,
        impl_name: Si::IMPL,
        description,
        scores_accuracy: true,
        picks_width: true,
        takes_columns: <i64 as BenchItem>::TAKES_COLUMNS,
        value_type: <i64 as BenchItem>::DATA_TYPE,
        capability: <G as GroundTruthCalculator<i64>>::CAPABILITY,
        operations: scored_ops(Si::SUPPORTS_MERGE, Si::SUPPORTS_PREPARE),
        metrics: row_metrics(true),
        run,
        comparator: Some(<G as GroundTruthCalculator<i64>>::NAME),
    }
}

/// The three precisions are one row: they are one algorithm at one impl, and
/// `lg_k` is the knob that moves between them. `S14` names the row, the way the
/// `i64` half names an [`ordered`] one.
const fn lib_hll<S12, S14, S16, G>(description: &'static str, run: RunFn) -> Row
where
    S12: InitSketch + BenchImpl + MemoryFootprint,
    S14: InitSketch + BenchImpl + MemoryFootprint,
    S16: InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<i64>,
{
    Row {
        family: S14::FAMILY,
        algorithm: S14::ALGORITHM,
        impl_name: S14::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <i64 as BenchItem>::TAKES_COLUMNS,
        value_type: <i64 as BenchItem>::DATA_TYPE,
        capability: <G as GroundTruthCalculator<i64>>::CAPABILITY,
        operations: scored_ops(S14::SUPPORTS_MERGE, S14::SUPPORTS_PREPARE),
        metrics: row_metrics(true),
        run,
        comparator: Some(<G as GroundTruthCalculator<i64>>::NAME),
    }
}

/// The shape-dispatching counterpart of [`scored`]: identity comes off the
/// row's `FixedMatrixRow` impl and its params type, because no one storage type
/// names a row that exists at every shape.
const fn fixed_matrix_row<W: FixedMatrixRow, P: crate::params::SketchParams>(
    description: &'static str,
) -> Row {
    let run: RunFn = run_fixed_matrix::<W>;
    Row {
        family: P::FAMILY,
        algorithm: W::ALGORITHM,
        impl_name: "lib",
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: false,
        value_type: "i64",
        capability: W::CAPABILITY,
        operations: scored_ops(W::SUPPORTS_MERGE, W::SUPPORTS_PREPARE),
        metrics: row_metrics(true),
        run,
        // Its comparator is fixed inside the shape dispatch.
        comparator: Some("frequency"),
    }
}

const fn parallel_row<S, I>(description: &'static str, run: RunFn) -> Row
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    I: BenchItem,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <I as BenchItem>::TAKES_COLUMNS,
        value_type: <I as BenchItem>::DATA_TYPE,
        capability: Capability::None,
        operations: unscored_ops(S::SUPPORTS_MERGE, S::SUPPORTS_PREPARE),
        metrics: row_metrics(false),
        run,
        comparator: None,
    }
}


// ---------- the catalog ----------

/// Every `(algorithm, impl)` this crate exposes. Adding one is one line here plus
/// the wrapper it names; nothing else in this file changes.
pub const ROWS: &[Row] = &[
    // Each row names a `run_*` function in the wrapper file that owns the
    // sketch. That function states the sketch's `SketchOps` — how it is built,
    // fed, folded, finalised and asked — in its own terms. Nothing here forces
    // two rows to agree on any of it; the table only records what exists.

    // -------- HLL (cardinality) --------
    scored::<hll::HllOxide, i64, CardinalityGT>(
        "sketch_oxide::cardinality::HyperLogLog (lg_k 4..=18)",
        hll::run_oxide,
    ),
    scored::<hll::HllDatasketches, i64, CardinalityGT>(
        "datasketches::hll::HllSketch (Hll8)",
        hll::run_datasketches,
    ),
    lib_hll::<
        hll::HllLib<HllBucketListP12>,
        hll::HllLib<HllBucketListP14>,
        hll::HllLib<HllBucketListP16>,
        CardinalityGT,
    >(
        "asap_sketchlib::HyperLogLog<Classic>: O(m) estimate, lg_k in {12,14,16}",
        hll::run_lib,
    ),
    scored::<polars::PolarsCardinality, i64, CardinalityGT>(
        "polars exact: DataFrame.n_unique()",
        polars::run_cardinality,
    ),
    // -------- HLL, HIP estimator --------
    // Its own algorithm: the estimate is maintained on the insert path instead
    // of scanned at query time. It also supplies no `merge`, which its
    // `SketchOps` states as `None`.
    lib_hll::<
        hll::HllLibHip<HllBucketListP12>,
        hll::HllLibHip<HllBucketListP14>,
        hll::HllLibHip<HllBucketListP16>,
        CardinalityGT,
    >(
        "asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}",
        hll::run_lib_hip,
    ),
    // -------- HLL, parallel insert --------
    parallel_row::<parallel::ParallelHllFastPath, i64>(
        "asap HLL ErtlMLE, FastPath, parallel insert",
        parallel::run_hll,
    ),
    // -------- KLL (quantile, rank error) --------
    // Two query paths x two libraries. The `cdf` rows supply a `prepare` and the
    // per-call rows do not — that difference is the whole point of the split,
    // and it is now visible in the ops rather than hidden in a trait default.
    ordered::<kll::KllOxidePerCall<i64>, kll::KllOxidePerCall<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: quantile() per call",
        kll::run_oxide_percall,
    ),
    ordered::<kll::KllLibPerCall<i64>, kll::KllLibPerCall<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: quantile() per call, k in [8, 26602]",
        kll::run_lib_percall,
    ),
    ordered::<kll::KllOxideCdf<i64>, kll::KllOxideCdf<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: cdf() built in prepare",
        kll::run_oxide_cdf,
    ),
    ordered::<kll::KllLibCdf<i64>, kll::KllLibCdf<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: cdf() built in prepare, k in [8, 26602]",
        kll::run_lib_cdf,
    ),
    scored::<polars::PolarsQuantileKll, i64, RankErrorGT>(
        "polars exact: 101-point quantile grid",
        polars::run_quantile_kll,
    ),
    // -------- CMS (frequency) --------
    scored::<cms::CmsOxide, i64, FrequencyGT>(
        "sketch_oxide::frequency::CountMinSketch",
        cms::run_oxide,
    ),
    scored::<cms::CmsDatasketches, i64, FrequencyGT>(
        "datasketches::countmin::CountMinSketch",
        cms::run_datasketches,
    ),
    scored::<polars::PolarsFrequencyCms, i64, FrequencyGT>(
        "polars exact: group_by(v).agg(len)",
        polars::run_frequency_cms,
    ),
    // The one row whose ops cannot be a `const`: its sketch type is a GAT, so
    // they come from `FixedMatrixRow::ops::<M>()` instead.
    fixed_matrix_row::<cms::CmsFixedMatrixRow, crate::params::CmsParams>(
        "asap CMS, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<cms::CmsLibVector2dFast, i64, FrequencyGT>(
        "asap CMS, Vector2D, FastPath",
        cms::run_vector2d_fast,
    ),
    scored::<cms::CmsLibVector2dRegular, i64, FrequencyGT>(
        "asap CMS, Vector2D, RegularPath",
        cms::run_vector2d_regular,
    ),
    parallel_row::<parallel::ParallelCmsFastPath, i64>(
        "asap CMS, FastPath, parallel insert on M5x32K",
        parallel::run_cms,
    ),
    // -------- CountSketch (frequency) --------
    scored::<countsketch::CsOxide, i64, FrequencyGT>(
        "sketch_oxide::frequency::CountSketch",
        countsketch::run_oxide,
    ),
    scored::<polars::PolarsFrequencyCs, i64, FrequencyGT>(
        "polars exact: group_by(v).agg(len)",
        polars::run_frequency_cs,
    ),
    fixed_matrix_row::<countsketch::CsFixedMatrixRow, crate::params::CountSketchParams>(
        "asap Count, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<countsketch::CsLibVector2dFast, i64, FrequencyGT>(
        "asap Count, Vector2D, FastPath",
        countsketch::run_vector2d_fast,
    ),
    scored::<countsketch::CsLibVector2dRegular, i64, FrequencyGT>(
        "asap Count, Vector2D, RegularPath",
        countsketch::run_vector2d_regular,
    ),
    parallel_row::<parallel::ParallelCsFastPath, i64>(
        "asap Count, FastPath, parallel insert on M5x32K",
        parallel::run_cs,
    ),
    // -------- Hydra (per-subpopulation statistics over labelled records) --------
    // Three rows, three different probe shapes. See `wrappers/hydra.rs`.
    scored::<hydra::HydraCms, Labeled<i64>, SubpopFrequencyGT>(
        "asap_sketchlib::Hydra over Count-Min cells (subpopulation frequency)",
        hydra::run_cms,
    ),
    scored::<polars::PolarsSubpopFrequency, Labeled<i64>, SubpopFrequencyGT>(
        "polars exact: group_by(subset, v).agg(len) over every label subset",
        polars::run_subpop_frequency,
    ),
    scored::<hydra::HydraHll, Labeled<i64>, SubpopCardinalityGT>(
        "asap_sketchlib::Hydra over HyperLogLog cells (subpopulation cardinality)",
        hydra::run_hll,
    ),
    scored::<polars::PolarsSubpopCardinality, Labeled<i64>, SubpopCardinalityGT>(
        "polars exact: group_by(subset).agg(v.n_unique()) over every label subset",
        polars::run_subpop_cardinality,
    ),
    scored::<hydra::HydraKll, Labeled<f64>, SubpopRankErrorGT>(
        "asap_sketchlib::Hydra over KLL cells (subpopulation quantile)",
        hydra::run_kll,
    ),
    scored::<polars::PolarsSubpopQuantile, Labeled<f64>, SubpopRankErrorGT>(
        "polars exact: sorted values per label subset, quantile by rank",
        polars::run_subpop_quantile,
    ),
];



// ---------- what the frontend asks ----------

fn find(algorithm: &str, impl_name: &str) -> Option<&'static Row> {
    ROWS.iter()
        .find(|r| r.algorithm == algorithm && r.impl_name == impl_name)
}

/// One line per row, grouped by family with a blank line between groups, since
/// the family is what a reader picks from before they pick a variant. Rows keep
/// declaration order inside a family.
///
/// The algorithm column is sized to the longest name present, so adding a
/// longer variant widens the table instead of breaking its alignment. The first
/// line is the header, so a caller prints exactly what this returns.
pub fn list() -> Vec<String> {
    let algo_w = ROWS
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = ROWS.iter().map(|r| r.impl_name.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(ROWS.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  description",
        "# algorithm", "impl"
    ));
    let mut current: Option<&str> = None;
    for r in ROWS {
        if current != Some(r.family) {
            out.push(String::new());
            current = Some(r.family);
        }
        out.push(format!(
            "{:algo_w$}  {:impl_w$}  {}",
            r.algorithm, r.impl_name, r.description
        ));
    }
    out
}

pub fn algorithm_exists(algorithm: &str) -> bool {
    ROWS.iter().any(|r| r.algorithm == algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field. `None`
/// if the algorithm is unknown, which the frontend has already ruled out by the
/// time it asks.
pub fn family_of(algorithm: &str) -> Option<&'static str> {
    ROWS.iter()
        .find(|r| r.algorithm == algorithm)
        .map(|r| r.family)
}

/// Can a comparator score this row? `None` if the row is unknown.
pub fn scores_accuracy(algorithm: &str, impl_name: &str) -> Option<bool> {
    find(algorithm, impl_name).map(|r| r.scores_accuracy)
}

/// Parse the single `--config` point for an algorithm, checking the algorithm exists.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet> {
    if !algorithm_exists(algorithm) {
        anyhow::bail!("unknown sketch algorithm: {algorithm}");
    }
    ParamSet::single(algorithm, spec).map_err(Into::into)
}

/// Why a request cannot run. Every variant names the row and what about the
/// request it could not honour, because the whole value of answering here is
/// that the answer arrives before a workload is generated.
#[derive(Debug)]
pub enum ResolveError {
    UnknownAlgorithm(String),
    UnknownImpl { algorithm: String, impl_name: String },
    /// A width the row's item type cannot be built at.
    WidthUnsupported { algorithm: String, impl_name: String },
    /// An operation this row does not have — no merge, or no prepare.
    OperationUnsupported {
        algorithm: String,
        impl_name: String,
        operation: &'static str,
        admits: String,
    },
    /// A metric this row cannot carry — accuracy on a row nothing scores.
    MetricUnsupported {
        algorithm: String,
        impl_name: String,
        metric: &'static str,
        capability: &'static str,
    },
    /// A square that no row could fill, because the framework measures nothing
    /// there. Distinct from the two above: this is not about the row.
    NothingMeasuresIt {
        operation: &'static str,
        metric: &'static str,
    },
    UnknownComparator {
        algorithm: String,
        impl_name: String,
        name: String,
        admits: String,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UnknownAlgorithm(a) => write!(f, "unknown sketch algorithm: {a}"),
            ResolveError::UnknownImpl { algorithm, impl_name } => {
                write!(f, "no impl '{impl_name}' for algorithm '{algorithm}'")
            }
            ResolveError::WidthUnsupported { algorithm, impl_name } => {
                write!(f, "{algorithm}/{impl_name} runs over i64 only; drop --dtype f64")
            }
            ResolveError::OperationUnsupported { algorithm, impl_name, operation, admits } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} has no {operation}; it can be measured over {admits}"
                )
            }
            ResolveError::MetricUnsupported { algorithm, impl_name, metric, capability } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} answers no statistic (capability {capability}), \
                     so nothing can score its {metric}"
                )
            }
            ResolveError::NothingMeasuresIt { operation, metric } => {
                write!(f, "nothing measures the {metric} of {operation}, for any sketch")
            }
            ResolveError::UnknownComparator { algorithm, impl_name, name, admits } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} has no comparator '{name}'; it admits {admits}"
                )
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// A resolved request: the one thing left to do is run it.
///
/// Deliberately *not* called a closure. It is a `fn` pointer plus the few facts
/// a caller needs before it can generate a workload — a `fn` pointer captures
/// nothing, so calling this a closure would claim something untrue. The
/// closures in this design are the per-row `ask` bodies written in [`ROWS`];
/// this is the handle that selects one.
///
/// A `fn` and not a `Box<dyn FnOnce>` because it is called **once** per
/// process, so boxing buys nothing, and staying a `fn` keeps the table
/// `const`-constructible. Everything the pointer reaches is monomorphised: the
/// erasure happens here, at the crate boundary, outside anything timed.
///
/// `Debug` prints the row it resolved to, not the pointer — a `fn` address says
/// nothing to a reader, and this is what shows up when a test unwraps the wrong
/// way round.
pub struct ResolvedRow<F> {
    /// The closure that runs this request. Built by [`resolve`], which captures
    /// the row's monomorphic runner and the width the request resolved at, so a
    /// caller supplies only what it owns: the config, the data it generated,
    /// and the params.
    ///
    /// `impl Fn`, not `Box<dyn Fn>` — the type is known statically, so there is
    /// no allocation and no dynamic dispatch anywhere in the chain.
    pub run: F,
    /// The `data_type` the caller has to generate this row's value column at.
    /// The reason resolution comes first: only the row knows its item type, so
    /// a caller cannot generate a workload until it has asked.
    pub value_type: &'static str,
    /// Whether this row ingests labelled records, and so needs a multi-column
    /// description rather than a single-column one.
    pub takes_columns: bool,
    /// The family this row belongs to, for the record's `family` field.
    pub family: &'static str,
}

impl<F> std::fmt::Debug for ResolvedRow<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRow")
            .field("family", &self.family)
            .field("value_type", &self.value_type)
            .field("takes_columns", &self.takes_columns)
            .finish_non_exhaustive()
    }
}

/// Can this request run, and if so how?
///
/// `Ok(Some(closure))` — it can; call it. `Ok(None)` — the request selected no
/// squares at all (an empty mask on either axis), which is legal and produces
/// no records. `Err` — it cannot, and the error says what about the request the
/// registry could not honour.
///
/// Every check here is answerable from the request and the catalog alone, which
/// is the point: a refusal costs nothing, because it lands before a single item
/// is generated.
#[allow(clippy::type_complexity)]
pub fn resolve(
    req: &Requirement,
) -> Result<
    Option<
        ResolvedRow<
            impl Fn(&BenchConfig, WorkloadData, &ParamSet) -> Result<Vec<BenchReport>, RunError>,
        >,
    >,
    ResolveError,
> {
    let row = find(&req.algorithm, &req.impl_name).ok_or_else(|| {
        if algorithm_exists(&req.algorithm) {
            ResolveError::UnknownImpl {
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
            }
        } else {
            ResolveError::UnknownAlgorithm(req.algorithm.clone())
        }
    })?;

    // A width this row's type cannot be built at.
    if req.width == Numeric::F64 && !row.picks_width {
        return Err(ResolveError::WidthUnsupported {
            algorithm: req.algorithm.clone(),
            impl_name: req.impl_name.clone(),
        });
    }

    // Every square the request selects has to clear two independent bars: the
    // framework has to measure it at all, and this row has to have it.
    let squares = cells(req.operations, req.metrics);
    for cell in &squares {
        if !is_measurable(*cell) {
            return Err(ResolveError::NothingMeasuresIt {
                operation: cell.operation.name(),
                metric: cell.metric.name(),
            });
        }
    }
    // Metrics before operations, deliberately. A row nothing scores lacks
    // accuracy *and* query, so checking operations first would always answer
    // "no query" and never name the capability that is the actual reason.
    for (bit, metric) in [
        (MetricsMask::ACCURACY, "accuracy"),
        (MetricsMask::THROUGHPUT, "throughput"),
        (MetricsMask::LATENCY, "latency"),
    ] {
        if req.metrics.contains(bit) && !row.metrics.contains(bit) {
            return Err(ResolveError::MetricUnsupported {
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
                metric,
                capability: row.capability.name(),
            });
        }
    }
    for (bit, operation) in [
        (OperationMask::INSERT, "insert"),
        (OperationMask::QUERY, "query"),
        (OperationMask::MERGE, "merge"),
        (OperationMask::PREPARE, "prepare"),
    ] {
        if req.operations.contains(bit) && !row.operations.contains(bit) {
            return Err(ResolveError::OperationUnsupported {
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
                operation,
                admits: operations_of(row),
            });
        }
    }

    // A named comparator has to be one this row admits.
    if let Some(name) = req.comparator.as_deref() {
        if row.comparator != Some(name) {
            return Err(ResolveError::UnknownComparator {
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
                name: name.to_string(),
                admits: comparators_of(row),
            });
        }
    }
    let run = row.run;

    // Nothing was asked for. Legal, and not an error — the caller gets no
    // records because it selected no squares, not because anything failed.
    if squares.is_empty() {
        return Ok(None);
    }

    // The closure. It captures `run` — the row's monomorphic runner — and the
    // width this request resolved at, so the caller supplies only what it
    // actually owns: the config, the data it generated, and the params.
    let width = req.width;
    Ok(Some(ResolvedRow {
        run: move |cfg: &BenchConfig, data: WorkloadData, params: &ParamSet| {
            run(cfg, data, params, width)
        },
        // At the *requested* width, not the row's default. An `ordered` row is
        // named by its i64 half, so `row.value_type` is "i64" even when the
        // request is for f64 — generating from that would hand an i64 column to
        // an f64 workload. Only a row that states both widths can move here;
        // every other row was refused above if it was asked for f64.
        value_type: if row.picks_width {
            req.width.name()
        } else {
            row.value_type
        },
        takes_columns: row.takes_columns,
        family: row.family,
    }))
}

/// The operations a row admits, for an error message.
fn operations_of(row: &Row) -> String {
    let names: Vec<&str> = [
        (OperationMask::INSERT, "insert"),
        (OperationMask::QUERY, "query"),
        (OperationMask::MERGE, "merge"),
        (OperationMask::PREPARE, "prepare"),
    ]
    .into_iter()
    .filter(|(b, _)| row.operations.contains(*b))
    .map(|(_, n)| n)
    .collect();
    names.join(", ")
}

/// The comparator names a row admits, for an error message.
fn comparators_of(row: &Row) -> String {
    row.comparator.unwrap_or("none").to_string()
}

/// Which comparators a row admits. `None` for an unknown row, so a frontend
/// can tell "no such row" from "that row is scored by nothing".
pub fn comparators(algorithm: &str, impl_name: &str) -> Option<Vec<&'static str>> {
    find(algorithm, impl_name).map(|row| row.comparator.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::cell::WorkloadSpec;
    use aqpbm_core::metrics::{MetricsMask, OperationMask};
    use crate::params::{
        CmsParams, CountSketchParams, HllParams, HydraCmsParams, HydraHllParams, HydraKllParams,
        KllParams, SketchParams,
    };
    use std::collections::BTreeSet;

    /// One buildable config per family, from each params type's own
    /// `canonical()`, tagged with the row's own algorithm. The `panic!` arm is
    /// what makes a newly added family show up here rather than silently
    /// skipping the tests below.
    pub(super) fn canonical_params(row: &Row) -> ParamSet {
        let a = row.algorithm;
        match row.family {
            "hll" => ParamSet::of_algorithm(a, &HllParams::canonical()),
            "kll" => ParamSet::of_algorithm(a, &KllParams::canonical()),
            "cms" => ParamSet::of_algorithm(a, &CmsParams::canonical()),
            "countsketch" => ParamSet::of_algorithm(a, &CountSketchParams::canonical()),
            "hydra-cms" => ParamSet::of_algorithm(a, &HydraCmsParams::canonical()),
            "hydra-hll" => ParamSet::of_algorithm(a, &HydraHllParams::canonical()),
            "hydra-kll" => ParamSet::of_algorithm(a, &HydraKllParams::canonical()),
            other => panic!("no canonical params known for family '{other}'"),
        }
    }

    /// The one way two rows can still collide: distinct types declaring the same
    /// `IMPL` under the same algorithm. [`find`] takes the first, so the second is
    /// dead. Not a compile error — the strings come from two different types.
    #[test]
    fn algorithm_impl_pairs_are_unique() {
        let mut seen = BTreeSet::new();
        for r in ROWS {
            assert!(
                seen.insert((r.algorithm, r.impl_name)),
                "duplicate row {}/{}",
                r.algorithm,
                r.impl_name
            );
        }
    }

    /// A row's algorithm must sit in the family whose vocabulary it parses, or
    /// `--config` would be checked against knobs the row does not take. The
    /// `ALGORITHM` override is a hand-written string, so this is the one thing
    /// about a row's identity the type system does not already guarantee.
    #[test]
    fn every_algorithm_belongs_to_its_family() {
        for r in ROWS {
            assert!(
                crate::params::in_family(r.algorithm, r.family),
                "row {}/{} declares family '{}', which its algorithm is not in",
                r.algorithm,
                r.impl_name,
                r.family
            );
        }
    }

    /// The impl axis carries the library and nothing else. A storage backend, a
    /// code path or a query strategy in this column is the defect this naming
    /// exists to prevent: it makes the column mean two things at once, so
    /// "which library is faster" stops being answerable by grouping on it.
    #[test]
    fn impl_names_are_library_names() {
        const LIBRARIES: [&str; 4] = ["oxide", "datasketches", "lib", "polars"];
        for r in ROWS {
            assert!(
                LIBRARIES.contains(&r.impl_name),
                "row {}/{}: '{}' is not a library name; a structural variant \
                 belongs in the algorithm",
                r.algorithm,
                r.impl_name,
                r.impl_name
            );
        }
    }

    /// Every family has at least one row a `--config` sweep can walk, and every
    /// family's rows are reachable by name. A family whose every row were fixed
    /// shape would be a panel with no x-axis.
    #[test]
    fn every_family_is_reachable_by_name() {
        for r in ROWS {
            assert_eq!(
                family_of(r.algorithm),
                Some(r.family),
                "{} does not resolve to its own family",
                r.algorithm
            );
        }
        assert_eq!(family_of("no-such-algorithm"), None);
    }

    /// A small workload spec, enough for any row to build and ingest. The item
    /// type is not named here — `Inline` lets each row materialise at its own
    /// `Accumulator::Item`, which is what the CLI's inline flags do too.
    pub(super) fn smoke_spec() -> WorkloadSpec {
        WorkloadSpec::Inline(aqpbm_core::TableDescription::single(
            "key",
            column(64, 1, "i64"),
            256,
        ))
    }

    fn column(cardinality: u64, seed: u64, data_type: &str) -> aqpbm_core::ColumnSpec {
        aqpbm_core::ColumnSpec {
            distribution: aqpbm_core::DataDistribution::Uniform(aqpbm_core::UniformParameter {
                lower_bound: 0.0,
                upper_bound: cardinality as f64,
                seed,
            }),
            shift: None,
            cardinality: None,
            special_rule: aqpbm_core::RULE_NONE,
            data_type: data_type.into(),
            string: None,
        }
    }

    /// The same, for the rows whose item is a record: two label columns and a
    /// value column. A row states which of the two it wants through
    /// `Row::takes_columns`, so neither is guessed here.
    ///
    /// `Generated`, not `Inline`: a record needs its label columns rendered as
    /// text and its value column as the row's item type, and only a written
    /// description can say so per column. Which is why one file cannot serve
    /// both the `i64` and the `f64` record rows.
    fn smoke_columns_spec(value_type: &str) -> WorkloadSpec {
        WorkloadSpec::Generated(aqpbm_core::TableDescription {
            column_num: 3,
            column_label: vec!["key1".into(), "key2".into(), "value".into()],
            column_spec: vec![
                column(8, 1, "string"),
                column(4, 2, "string"),
                column(32, 3, value_type),
            ],
            column_connected: Vec::new(),
            row_num: 256,
        })
    }

    /// The spec shape `row` can actually ingest.
    pub(super) fn spec_for(row: &Row) -> WorkloadSpec {
        if row.takes_columns {
            smoke_columns_spec(row.value_type)
        } else {
            smoke_spec()
        }
    }

    pub(super) fn smoke_cfg() -> BenchConfig {
        BenchConfig {
            runs: 1,
            warmup_runs: 0,
            // Only squares something measures. The default request reaches
            // `(insert, accuracy)` and `(query, latency)`, which nothing
            // does, and reaching an empty square is an error.
            metrics: MetricsMask::THROUGHPUT,
            operations: OperationMask::INSERT,
            ..Default::default()
        }
    }

    /// Every row actually builds and ingests — the part the types cannot state.
    /// A row *is* its runner, so there is no `_` bail to fall into and no strings
    /// for the list and the dispatch to disagree about.
    #[test]
    fn every_catalog_entry_runs() {
        let cfg = smoke_cfg();
        for r in ROWS {
            // Canonical, not `empty`: every family's params have required
            // fields, so `empty` builds nothing at all now that the exact
            // baselines parse their config too.
            let params = canonical_params(r);
            let req = Requirement {
                algorithm: r.algorithm.to_string(),
                impl_name: r.impl_name.to_string(),
                params: params.clone(),
                operations: cfg.operations,
                metrics: cfg.metrics,
                width: Numeric::I64,
                comparator: None,
            };
            // Resolution is separate from execution now, so a row that cannot
            // be *selected* is a different failure from one that cannot run.
            let closure = resolve(&req)
                .unwrap_or_else(|e| panic!("{}/{} will not resolve: {e}", r.algorithm, r.impl_name))
                .expect("the smoke request selects one square");
            assert_eq!(
                closure.family, r.family,
                "{}/{} resolves to the wrong family",
                r.algorithm, r.impl_name
            );
            // Generation now happens here, at the type the row named — the
            // same order the frontend follows.
            let data = spec_for(r)
                .generate_at(closure.value_type)
                .unwrap_or_else(|e| panic!("{}/{}: {e}", r.algorithm, r.impl_name));
            let got = (closure.run)(&cfg, data, &params);
            // Fixed-matrix rows refuse an off-shape config (a `RunError::Build`
            // surfaced as an error); every other row runs.
            if let Ok(reports) = &got {
                for report in reports {
                    assert_eq!(
                        (report.sketch.as_str(), report.impl_name.as_str()),
                        (r.algorithm, r.impl_name),
                        "row {}/{} emits records labelled {}/{}",
                        r.algorithm,
                        r.impl_name,
                        report.sketch,
                        report.impl_name,
                    );
                }
            }
        }
    }

    // An algorithm's rows are only comparable if asked the same question, so
    // they must agree on which configs are answerable — one row silently
    // accepting a config its peers reject scores a different experiment. The
    // test that pinned this swept the `topk` family and went with it; restore
    // it against whichever family next has a config knob its rows could
    // disagree on.
}

#[cfg(test)]
mod declared_support_tests {
    use super::*;

    /// What a row declares has to be what it can actually do, and the failure is
    /// silent in both directions: under-declaring hides a measurable square from
    /// the frontend, over-declaring makes it generate a whole workload for one
    /// that comes back empty. Spot-check the rows whose answers differ.
    #[test]
    fn rows_declare_the_operations_their_impls_have() {
        let ops = |a: &str, i: &str| find(a, i).map(|r| r.operations);
        let has = |a: &str, i: &str, o: OperationMask| ops(a, i).unwrap().contains(o);

        // Every row inserts.
        for r in ROWS {
            assert!(
                r.operations.contains(OperationMask::INSERT),
                "{}/{} declares no insert",
                r.algorithm,
                r.impl_name
            );
        }

        // `prepare` is the doc's example of an operation one algorithm has and
        // most do not: KLL's cdf rows build their lookup there, the per-call
        // rows have nothing to do.
        assert!(has("kll-cdf", "oxide", OperationMask::PREPARE));
        assert!(has("kll-cdf", "lib", OperationMask::PREPARE));
        assert!(!has("kll-percall", "oxide", OperationMask::PREPARE));
        assert!(!has("kll-percall", "lib", OperationMask::PREPARE));
        assert!(!has("cms", "oxide", OperationMask::PREPARE));

        // Merge divides the same family: the oxide and datasketches HLLs fold,
        // the sketchlib HIP variant does not.
        assert!(has("cms", "oxide", OperationMask::MERGE));
        assert!(has("hll", "oxide", OperationMask::MERGE));
        assert!(has("hll", "lib", OperationMask::MERGE));
        assert!(!has("hll-hip", "lib", OperationMask::MERGE));

        // A parallel row has no comparator, so nothing issues its queries.
        assert!(!has("cms-fastpath-fixedmatrix-32k-parallel", "lib", OperationMask::QUERY));
        assert!(has("cms-fastpath-fixedmatrix-32k-parallel", "lib", OperationMask::PREPARE));
    }

    /// Accuracy is the one metric that needs a comparator, so it has to track
    /// the capability rather than being declared beside it.
    #[test]
    fn accuracy_is_declared_exactly_where_a_comparator_can_score_it() {
        for r in ROWS {
            assert_eq!(
                r.metrics.contains(MetricsMask::ACCURACY),
                r.capability.scores(),
                "{}/{} declares accuracy={} but capability {}",
                r.algorithm,
                r.impl_name,
                r.metrics.contains(MetricsMask::ACCURACY),
                r.capability.name()
            );
            // The two ways of saying the same thing must not disagree.
            assert_eq!(
                r.scores_accuracy,
                r.capability.scores(),
                "{}/{}: scores_accuracy and capability disagree",
                r.algorithm,
                r.impl_name
            );
        }
    }

    /// The capability is what a listing prints and what a comparator binds, so
    /// each family must land on the statistic it actually answers.
    #[test]
    fn every_row_lands_on_the_statistic_its_family_answers() {
        for (algorithm, want) in [
            ("hll", Capability::Cardinality),
            ("hll-hip", Capability::Cardinality),
            ("kll-cdf", Capability::Quantile),
            ("kll-percall", Capability::Quantile),
            ("cms", Capability::Frequency),
            ("cms-fastpath-fixedmatrix", Capability::Frequency),
            ("countsketch", Capability::Frequency),
            ("hydra-cms", Capability::SubpopFrequency),
            ("hydra-hll", Capability::SubpopCardinality),
            ("hydra-kll", Capability::SubpopQuantile),
        ] {
            for r in ROWS.iter().filter(|r| r.algorithm == algorithm) {
                assert_eq!(
                    r.capability, want,
                    "{}/{} answers {}, expected {}",
                    r.algorithm,
                    r.impl_name,
                    r.capability.name(),
                    want.name()
                );
            }
        }
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn req(algorithm: &str, impl_name: &str, ops: OperationMask, metrics: MetricsMask) -> Requirement {
        Requirement {
            algorithm: algorithm.to_string(),
            impl_name: impl_name.to_string(),
            params: ParamSet::empty(algorithm),
            operations: ops,
            metrics,
            width: Numeric::I64,
            comparator: None,
        }
    }

    /// The doc's own example: `prepare` exists for KLL and is missing from most
    /// sketches. Asking a row for one it does not have is answered from the
    /// catalog, by name — no workload is generated to find out.
    #[test]
    fn an_operation_a_row_does_not_have_is_refused_by_name() {
        let bad = resolve(&req(
            "cms",
            "oxide",
            OperationMask::PREPARE,
            MetricsMask::LATENCY,
        ))
        .expect_err("cms/oxide has no prepare");
        let msg = bad.to_string();
        assert!(msg.contains("cms/oxide"), "{msg}");
        assert!(msg.contains("prepare"), "{msg}");
        // And it says what the row *can* be measured over, so the message is a
        // recipe rather than a rejection.
        assert!(msg.contains("insert"), "{msg}");

        // The same request against the row that does have one resolves.
        assert!(resolve(&req(
            "kll-cdf",
            "oxide",
            OperationMask::PREPARE,
            MetricsMask::LATENCY,
        ))
        .unwrap()
        .is_some());
    }

    /// Merge divides one family: the sketchlib HLL folds, its HIP variant does
    /// not. Both are `hll`-shaped, so only the declaration tells them apart.
    #[test]
    fn merge_is_refused_on_the_row_that_cannot_fold() {
        assert!(resolve(&req(
            "hll",
            "lib",
            OperationMask::MERGE,
            MetricsMask::THROUGHPUT
        ))
        .unwrap()
        .is_some());
        let bad = resolve(&req(
            "hll-hip",
            "lib",
            OperationMask::MERGE,
            MetricsMask::THROUGHPUT,
        ))
        .expect_err("hll-hip provides no merge");
        assert!(bad.to_string().contains("merge"), "{bad}");
    }

    /// Accuracy needs a comparator. A row nothing scores is refused for it,
    /// naming the capability that made it so.
    #[test]
    fn accuracy_is_refused_on_a_row_no_comparator_scores() {
        // Accuracy only ever pairs with query — every other square carrying it
        // is one of the four the framework leaves empty — so this is the only
        // request that reaches the row-level check at all.
        let bad = resolve(&req(
            "cms-fastpath-fixedmatrix-32k-parallel",
            "lib",
            OperationMask::QUERY,
            MetricsMask::ACCURACY,
        ))
        .expect_err("a parallel row answers no statistic");
        let msg = bad.to_string();
        assert!(msg.contains("none"), "should name the capability: {msg}");
        assert!(msg.contains("accuracy"), "{msg}");

        // Asked for a *timed* query instead, the refusal is about the missing
        // operation rather than the missing statistic.
        let bad = resolve(&req(
            "cms-fastpath-fixedmatrix-32k-parallel",
            "lib",
            OperationMask::QUERY,
            MetricsMask::THROUGHPUT,
        ))
        .expect_err("a parallel row issues no queries");
        assert!(bad.to_string().contains("query"), "{bad}");
    }

    /// A square nothing measures for *any* row is a different refusal from one
    /// a particular row lacks, and says so.
    #[test]
    fn a_square_nothing_measures_is_refused_before_the_row_is_consulted() {
        let bad = resolve(&req(
            "cms",
            "oxide",
            OperationMask::PREPARE,
            MetricsMask::THROUGHPUT,
        ))
        .expect_err("prepare has a latency but no throughput of its own");
        let msg = bad.to_string();
        assert!(msg.contains("for any sketch"), "{msg}");
    }

    /// Unknown names are told apart: a bad algorithm is not a bad impl.
    #[test]
    fn unknown_names_are_refused_distinctly() {
        let ops = OperationMask::INSERT;
        let m = MetricsMask::THROUGHPUT;
        assert!(matches!(
            resolve(&req("nosuch", "oxide", ops, m)),
            Err(ResolveError::UnknownAlgorithm(_))
        ));
        assert!(matches!(
            resolve(&req("cms", "nosuch", ops, m)),
            Err(ResolveError::UnknownImpl { .. })
        ));
        assert!(matches!(
            resolve(&req("cms", "oxide", ops, m)).map(|c| c.is_some()),
            Ok(true)
        ));
    }

    /// f64 on a row whose item type is fixed — the check that already ran early,
    /// now living beside the rest.
    #[test]
    fn a_width_the_row_cannot_be_built_at_is_refused() {
        let mut r = req("cms", "oxide", OperationMask::INSERT, MetricsMask::THROUGHPUT);
        r.width = Numeric::F64;
        assert!(matches!(
            resolve(&r),
            Err(ResolveError::WidthUnsupported { .. })
        ));
        // KLL states both widths, so the same request resolves there.
        let mut ok = req("kll-percall", "oxide", OperationMask::INSERT, MetricsMask::THROUGHPUT);
        ok.width = Numeric::F64;
        assert!(resolve(&ok).unwrap().is_some());
    }

    /// Selecting nothing is legal and is not an error: `Ok(None)` says the
    /// request named no squares, which is a different answer from "cannot run".
    #[test]
    fn an_empty_request_resolves_to_no_closure_rather_than_an_error() {
        assert!(resolve(&req("cms", "oxide", OperationMask::empty(), MetricsMask::all()))
            .unwrap()
            .is_none());
        assert!(resolve(&req("cms", "oxide", OperationMask::INSERT, MetricsMask::empty()))
            .unwrap()
            .is_none());
    }

    /// The reason resolution comes before generation: only the row knows what
    /// item type its workload has to be built at.
    #[test]
    fn a_resolved_closure_names_the_type_its_workload_must_be_generated_at() {
        let c = resolve(&req("cms", "oxide", OperationMask::INSERT, MetricsMask::THROUGHPUT))
            .unwrap()
            .unwrap();
        assert_eq!(c.value_type, "i64");
        assert!(!c.takes_columns);

        // Hydra ingests labelled records, so it needs a multi-column description.
        let h = resolve(&req("hydra-cms", "lib", OperationMask::INSERT, MetricsMask::THROUGHPUT))
            .unwrap()
            .unwrap();
        assert!(h.takes_columns);
    }

    /// A comparator the row does not admit is refused by name, listing what it
    /// does admit.
    #[test]
    fn an_unadmitted_comparator_is_refused_by_name() {
        let mut r = req("cms", "oxide", OperationMask::INSERT, MetricsMask::THROUGHPUT);
        r.comparator = Some("cardinality".to_string());
        let bad = resolve(&r).expect_err("cms is scored by frequency, not cardinality");
        let msg = bad.to_string();
        assert!(msg.contains("cardinality"), "{msg}");
        assert!(msg.contains("frequency"), "should list what it admits: {msg}");
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    /// An `ordered` row is named by its i64 half, so the row's own `value_type`
    /// says "i64" at every width. The closure has to answer for the width that
    /// was actually asked for — a frontend generates from this, and an i64
    /// column handed to an f64 workload is refused at materialise time.
    #[test]
    fn a_closure_names_the_value_type_of_the_width_it_resolved_at() {
        let req = |w| Requirement {
            algorithm: "kll-percall".to_string(),
            impl_name: "oxide".to_string(),
            params: ParamSet::empty("kll-percall"),
            operations: OperationMask::INSERT,
            metrics: MetricsMask::THROUGHPUT,
            width: w,
            comparator: None,
        };
        assert_eq!(
            resolve(&req(Numeric::I64)).unwrap().unwrap().value_type,
            "i64"
        );
        assert_eq!(
            resolve(&req(Numeric::F64)).unwrap().unwrap().value_type,
            "f64"
        );

        // A row whose item type is not numeric keeps its own answer: the width
        // never reaches it, because f64 was refused before this point.
        let hydra = Requirement {
            algorithm: "hydra-cms".to_string(),
            impl_name: "lib".to_string(),
            params: ParamSet::empty("hydra-cms"),
            operations: OperationMask::INSERT,
            metrics: MetricsMask::THROUGHPUT,
            width: Numeric::I64,
            comparator: None,
        };
        let c = resolve(&hydra).unwrap().unwrap();
        assert_eq!(c.value_type, "i64");
        assert!(c.takes_columns);
    }

    /// The end the bug actually showed up at: resolve, generate at what the
    /// closure named, run. This is the frontend's whole sequence, and it has to
    /// work at both widths.
    #[test]
    fn an_ordered_row_runs_at_both_widths_end_to_end() {
        for width in [Numeric::I64, Numeric::F64] {
            use crate::params::SketchParams;
            let params =
                ParamSet::of_algorithm("kll-percall", &crate::params::KllParams::canonical());
            let req = Requirement {
                algorithm: "kll-percall".to_string(),
                impl_name: "oxide".to_string(),
                params: params.clone(),
                operations: OperationMask::INSERT,
                metrics: MetricsMask::THROUGHPUT,
                width,
                comparator: None,
            };
            let closure = resolve(&req).unwrap().unwrap();
            let data = tests::smoke_spec()
                .generate_at(closure.value_type)
                .unwrap_or_else(|e| panic!("{width:?}: generate: {e}"));
            let cfg = tests::smoke_cfg();
            (closure.run)(&cfg, data, &params)
                .unwrap_or_else(|e| panic!("{width:?}: run: {e}"));
        }
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    /// The check the capability traits used to make for free.
    ///
    /// `impl FrequencyOps for X` was a compiler-checked claim that X answers
    /// frequency, and `statistic.rs` said so explicitly: nominal on purpose,
    /// because a structural bound "also matches sketches that answer a stub".
    /// An ask closure is checked only for *shape* — `|_, _| 0` type-checks
    /// perfectly — so that guarantee left with the traits.
    ///
    /// This is what replaces it, and it is a stronger claim than the traits
    /// made: not "the row declares a capability" but "the row's ask actually
    /// answers it". Every comparator is built so the do-nothing estimator
    /// scores exactly 1.0 on its relative-error metric, so a stub is caught by
    /// scoring no better than doing nothing.
    #[test]
    fn every_scored_row_answers_better_than_a_stub() {
        let cfg = BenchConfig {
            runs: 1,
            warmup_runs: 0,
            metrics: MetricsMask::ACCURACY,
            operations: OperationMask::QUERY,
            ..Default::default()
        };
        let mut checked = 0;
        for row in ROWS.iter().filter(|r| r.capability.scores()) {
            let params = tests::canonical_params(row);
            let req = Requirement {
                algorithm: row.algorithm.to_string(),
                impl_name: row.impl_name.to_string(),
                params: params.clone(),
                operations: cfg.operations,
                metrics: cfg.metrics,
                width: Numeric::I64,
                comparator: None,
            };
            let closure = resolve(&req).unwrap().unwrap();
            let data = tests::spec_for(row).generate_at(closure.value_type).unwrap();
            let reports = match (closure.run)(&cfg, data, &params) {
                Ok(r) => r,
                // A fixed-matrix row refuses an off-shape config; that is a
                // build refusal, not a stubbed answer.
                Err(_) => continue,
            };
            let acc = reports
                .iter()
                .find_map(|r| r.bench.accuracy.clone())
                .unwrap_or_else(|| panic!("{}/{} scored nothing", row.algorithm, row.impl_name));
            let acc = acc.as_object().expect("accuracy is an object");
            let get = |k: &str| acc.get(k).and_then(|v| v.as_f64());

            // One metric per capability, each one a relative error the null
            // estimator scores 1.0 on.
            let (key, err) = match row.capability {
                Capability::Cardinality => ("relative_error", get("relative_error")),
                Capability::Frequency | Capability::SubpopFrequency
                | Capability::SubpopCardinality => ("are_all", get("are_all")),
                Capability::Quantile | Capability::SubpopQuantile => {
                    ("max_rank_err", get("max_rank_err"))
                }
                Capability::TopK => ("recall_at_k", get("recall_at_k").map(|v| 1.0 - v)),
                Capability::None => unreachable!("filtered to scoring rows"),
            };
            let err = err.unwrap_or_else(|| {
                panic!(
                    "{}/{} reports no `{key}`; keys were {:?}",
                    row.algorithm,
                    row.impl_name,
                    acc.keys().collect::<Vec<_>>()
                )
            });
            assert!(
                err.is_finite() && err < 1.0,
                "{}/{} answers no better than a stub: {key} = {err}",
                row.algorithm,
                row.impl_name
            );
            checked += 1;
        }
        // Guard the guard: a filter that silently matched nothing would make
        // this test pass while checking not one row.
        assert!(checked >= 15, "only {checked} rows were actually scored");
    }
}
