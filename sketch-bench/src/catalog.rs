//! The catalog: one table naming every `(algorithm, impl)` this crate exposes,
//! and the dispatch resolving one to a concrete sketch type. It lives here,
//! not in the CLI, so a future `aqp-bench` can ship its own. A row is a
//! *type*, not a pair of strings — algorithm, impl name and `scores_accuracy`
//! are projected off it, so the list and the code cannot drift apart.

use anyhow::Result;
use aqpbm_core::accumulator::Accumulator;

use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::{self, AccuracyCfg, BenchItem, ParallelInit, RunError, WorkloadSpec};
use aqpbm_core::runner::NoGT;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::runner::{BenchConfig, BenchReport};

use asap_sketchlib::{
    DefaultXxHasher, FastPathHasher, HllBucketListP12, HllBucketListP14, HllBucketListP16,
    MatrixStorage,
};

use aqpbm_core::accuracy::FrequencyOps;

use crate::params::{HllParams, ParamSet, TopkParams};
use crate::wrappers::{
    cms, countsketch, dd, elastic, fixed_matrix, hll, hydra, kll, nitro, parallel, polars, topk,
    univmon,
};

// ---------- what a row is ----------

/// The one item-type choice a user still makes: an `ordered` row (KLL, DDSketch)
/// builds at either width, while every other row's item type is fixed by its Rust
/// type — so the catalog can refuse before generating anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Numeric {
    #[default]
    I64,
    F64,
}

/// The executable half of a row: everything the frontend can hand a cell.
type RunFn = fn(
    &BenchConfig,
    &WorkloadSpec,
    &ParamSet,
    &AccuracyCfg,
    Numeric,
) -> Result<Vec<BenchReport>, RunError>;

/// The comparators a row can be scored by, keyed by the name `--comparator`
/// selects them with.
type Comparators = &'static [(&'static str, RunFn)];

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
    /// Does `--accuracy` score this row? Derived: true iff it was built with a
    /// constructor that takes a ground-truth calculator.
    pub scores_accuracy: bool,
    /// Can this row run at [`Numeric::F64`]? Derived: only `ordered` rows can.
    pub picks_width: bool,
    /// Does this row ingest labelled records, and so need a `--spec` column
    /// list instead of a single-column spec? Derived off the row's item type.
    pub takes_columns: bool,
    run: RunFn,
    /// The comparators this row admits, by name, first one the default. Every
    /// entry is checked by the compiler: a calculator the row's capabilities
    /// cannot satisfy will not build, so the table cannot offer a comparison
    /// the row could not answer.
    comparators: Comparators,
}

// ---------- how a row builds its ground truth ----------

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* in [`ROWS`] and the row stays `const`.
trait GroundTruthCalculator<S: Accumulator>: GroundTruth<S> {
    /// The name `--comparator` selects this one by. One capability can carry
    /// several comparators, and this is what tells them apart on the command
    /// line.
    const NAME: &'static str;
    fn build(acc: &AccuracyCfg, params: &ParamSet) -> Self;
}

impl<S: Accumulator> GroundTruthCalculator<S> for CardinalityGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "cardinality";
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        CardinalityGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for FrequencyGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "frequency";
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        FrequencyGT {
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopFrequencyGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "subpop-frequency";
    /// Scores column 0. A grouped sketch stores every column subset, but each
    /// one is its own population with its own error, so a comparator names the
    /// one it scores instead of pooling them.
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopFrequencyGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopCardinalityGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "subpop-cardinality";
    /// Column 0, for the same reason as [`SubpopFrequencyGT`].
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopCardinalityGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopRankErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "subpop-rank-error";
    /// Column 0, and `max_probes` caps groups instead of keys: this comparator
    /// issues 101 estimate calls per group, so the cap bites much sooner here
    /// than it does for the two counting comparators.
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopRankErrorGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RankErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "rank-error";
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        RankErrorGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RelativeErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "relative-error";
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        RelativeErrorGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for TopkGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "topk";
    /// Scores against the same `k` the sketch was built with — a different prefix
    /// would measure the mismatch, not the sketch. Infallible because the timed
    /// half runs first, so an unreadable `k` has already failed the build.
    fn build(_acc: &AccuracyCfg, params: &ParamSet) -> Self {
        TopkGT {
            k: params
                .parse::<TopkParams>()
                .expect("the row built, so its params parse")
                .k,
        }
    }
}

// ---------- the ways a row runs ----------

/// Timed measurement, plus accuracy scored against `G` when `--accuracy` is on.
fn run_scored<S, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    // One walk of the grid: the squares needing a comparator run when there is
    // one, and are skipped when there is not.
    let gt = acc.enabled.then(|| G::build(acc, params));
    Ok(cell::run_cell::<S, G>(cfg, spec, params, gt.as_ref())?)
}

/// An ordered quantile algorithm (KLL, DDSketch): the row names both widths and
/// the caller's [`Numeric`] picks one. The only place a runtime value still
/// selects an item type.
fn run_ordered<Si, Sf, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    match width {
        Numeric::I64 => run_scored::<Si, G>(cfg, spec, params, acc, width),
        Numeric::F64 => run_scored::<Sf, G>(cfg, spec, params, acc, width),
    }
}

/// A row with no query capability: timed only, no ground truth, nothing to score.
fn run_plain<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell::<S, NoGT>(cfg, spec, params, None)
}

/// A parallel-insert row: built with the worker count, so not an `InitSketch`.
fn run_parallel<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell_parallel::<S, NoGT>(cfg, spec, params, None)
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
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    let (rows, cols) = W::shape(params)?;
    let visitor = RunFixedMatrix::<W> {
        cfg,
        spec,
        params,
        acc,
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
    /// The row's concrete type at storage `M`, which is what actually runs.
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    >: Accumulator<Item = i64>
        + InitSketch
        + BenchImpl
        + MemoryFootprint
        + FrequencyOps<Key = i64>;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    cfg: &'a BenchConfig,
    spec: &'a WorkloadSpec,
    params: &'a ParamSet,
    acc: &'a AccuracyCfg,
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
        run_scored::<W::At<M>, FrequencyGT>(self.cfg, self.spec, self.params, self.acc, self.width)
    }
}

/// A row whose construction parameter selects a *type* rather than a field.
/// `asap_sketchlib` puts the HLL register count in the storage type, so `lg_k`
/// picks a monomorphisation and this dispatch is what turns a runtime value
/// back into one. Same shape as [`run_ordered`], and for the same reason: an
/// enum inside the wrapper would put a branch in `update`, on rows whose whole
/// purpose is to price that insert.
fn run_lib_hll<S12, S14, S16, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S12: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S14: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S16: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<S12> + GroundTruthCalculator<S14> + GroundTruthCalculator<S16>,
{
    let p: HllParams = params.parse().map_err(BuildError::from)?;
    match p.lg_k {
        12 => run_scored::<S12, G>(cfg, spec, params, acc, width),
        14 => run_scored::<S14, G>(cfg, spec, params, acc, width),
        16 => run_scored::<S16, G>(cfg, spec, params, acc, width),
        other => Err(RunError::Build(hll::unsupported_precision(other))),
    }
}

// ---------- the row constructors ----------
// Each reads `S::FAMILY` / `S::ALGORITHM` / `S::IMPL` off the type and fixes
// `scores_accuracy`. `const fn`, so `ROWS` stays `const` and a bad row fails at
// compile time.

const fn scored<S, G>(description: &'static str) -> Row
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_scored::<S, G>,
        comparators: &[(G::NAME, run_scored::<S, G>)],
    }
}

const fn ordered<Si, Sf, G>(description: &'static str) -> Row
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    Row {
        // Both halves are the same row; the i64 one names it.
        family: Si::FAMILY,
        algorithm: Si::ALGORITHM,
        impl_name: Si::IMPL,
        description,
        scores_accuracy: true,
        picks_width: true,
        takes_columns: <Si::Item as BenchItem>::TAKES_COLUMNS,
        run: run_ordered::<Si, Sf, G>,
        comparators: &[(
            <G as GroundTruthCalculator<Si>>::NAME,
            run_ordered::<Si, Sf, G>,
        )],
    }
}

/// The three precisions are one row: they are one algorithm at one impl, and
/// `lg_k` is the knob that moves between them. `S14` names the row, the way the
/// `i64` half names an [`ordered`] one.
const fn lib_hll<S12, S14, S16, G>(description: &'static str) -> Row
where
    S12: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S14: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S16: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<S12> + GroundTruthCalculator<S14> + GroundTruthCalculator<S16>,
{
    Row {
        family: S14::FAMILY,
        algorithm: S14::ALGORITHM,
        impl_name: S14::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <S14::Item as BenchItem>::TAKES_COLUMNS,
        run: run_lib_hll::<S12, S14, S16, G>,
        comparators: &[(
            <G as GroundTruthCalculator<S12>>::NAME,
            run_lib_hll::<S12, S14, S16, G>,
        )],
    }
}

/// The shape-dispatching counterpart of [`scored`]: identity comes off the
/// row's `FixedMatrixRow` impl and its params type, because no one storage type
/// names a row that exists at every shape.
const fn fixed_matrix_row<W: FixedMatrixRow, P: crate::params::SketchParams>(
    description: &'static str,
) -> Row {
    Row {
        family: P::FAMILY,
        algorithm: W::ALGORITHM,
        impl_name: "lib",
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: false,
        run: run_fixed_matrix::<W>,
        // The fixed-matrix rows carry their comparator inside the generated
        // dispatch, so there is no calculator type here to name.
        comparators: &[],
    }
}

const fn plain<S>(description: &'static str) -> Row
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_plain::<S>,
        // Answers no query, so nothing scores it.
        comparators: &[],
    }
}

const fn parallel_row<S>(description: &'static str) -> Row
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_parallel::<S>,
        comparators: &[],
    }
}

// ---------- the catalog ----------

/// Every `(algorithm, impl)` this crate exposes. Adding one is one line here plus
/// the wrapper it names; nothing else in this file changes.
pub const ROWS: &[Row] = &[
    // -------- HLL (cardinality) --------
    scored::<hll::HllOxide, CardinalityGT>("sketch_oxide::cardinality::HyperLogLog (lg_k 4..=18)"),
    scored::<hll::HllDatasketches, CardinalityGT>("datasketches::hll::HllSketch (Hll8)"),
    lib_hll::<
        hll::HllLib<HllBucketListP12>,
        hll::HllLib<HllBucketListP14>,
        hll::HllLib<HllBucketListP16>,
        CardinalityGT,
    >("asap_sketchlib::HyperLogLog<Classic>: O(m) estimate, lg_k in {12,14,16}"),
    scored::<polars::PolarsCardinality, CardinalityGT>("polars exact: DataFrame.n_unique()"),
    // -------- HLL, HIP estimator --------
    // Its own algorithm: the estimate is maintained on the insert path instead
    // of scanned at query time, so it is different arithmetic and a different
    // number, not a different implementation of one number.
    lib_hll::<
        hll::HllLibHip<HllBucketListP12>,
        hll::HllLibHip<HllBucketListP14>,
        hll::HllLibHip<HllBucketListP16>,
        CardinalityGT,
    >("asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}"),
    // -------- HLL, parallel insert --------
    parallel_row::<parallel::ParallelHllFastPath>("asap HLL ErtlMLE, FastPath, parallel insert"),
    // -------- KLL (quantile, rank error) --------
    // Two query paths × two libraries. The paths answer differently, so the path
    // names the algorithm and each algorithm holds the two libraries against
    // each other. One row per library would have compared libraries and query
    // strategies in the same column.
    ordered::<kll::KllOxidePerCall<i64>, kll::KllOxidePerCall<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: quantile() per call",
    ),
    ordered::<kll::KllLibPerCall<i64>, kll::KllLibPerCall<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: quantile() per call, k in [8, 26602]",
    ),
    ordered::<kll::KllOxideCdf<i64>, kll::KllOxideCdf<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: cdf() built in prepare",
    ),
    ordered::<kll::KllLibCdf<i64>, kll::KllLibCdf<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: cdf() built in prepare, k in [8, 26602]",
    ),
    scored::<polars::PolarsQuantileKll, RankErrorGT>("polars exact: 101-point quantile grid"),
    // -------- CMS (frequency) --------
    scored::<cms::CmsOxide, FrequencyGT>("sketch_oxide::frequency::CountMinSketch"),
    scored::<cms::CmsDatasketches, FrequencyGT>("datasketches::countmin::CountMinSketch"),
    scored::<polars::PolarsFrequencyCms, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    fixed_matrix_row::<cms::CmsFixedMatrixRow, crate::params::CmsParams>(
        "asap CMS, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<cms::CmsLibVector2dFast, FrequencyGT>("asap CMS, Vector2D, FastPath"),
    scored::<cms::CmsLibVector2dRegular, FrequencyGT>("asap CMS, Vector2D, RegularPath"),
    parallel_row::<parallel::ParallelCmsFastPath>("asap CMS, FastPath, parallel insert on M5x32K"),
    // -------- CountSketch (frequency) --------
    scored::<countsketch::CsOxide, FrequencyGT>("sketch_oxide::frequency::CountSketch"),
    scored::<polars::PolarsFrequencyCs, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    fixed_matrix_row::<countsketch::CsFixedMatrixRow, crate::params::CountSketchParams>(
        "asap Count, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<countsketch::CsLibVector2dFast, FrequencyGT>("asap Count, Vector2D, FastPath"),
    scored::<countsketch::CsLibVector2dRegular, FrequencyGT>("asap Count, Vector2D, RegularPath"),
    parallel_row::<parallel::ParallelCsFastPath>("asap Count, FastPath, parallel insert on M5x32K"),
    // -------- DDSketch (quantile, relative error) --------
    ordered::<dd::DdLib<i64>, dd::DdLib<f64>, RelativeErrorGT>(
        "asap_sketchlib::DDSketch (relative-error quantile)",
    ),
    scored::<polars::PolarsQuantileDd, RelativeErrorGT>("polars exact: 101-point quantile grid"),
    // -------- Top-k (counter array + size-k candidate tracker) --------
    scored::<topk::TopKHeap<cms::CmsOxide>, TopkGT>(
        "sketch_oxide CMS + size-k heap (top-k on the insert path)",
    ),
    scored::<topk::TopKHeap<countsketch::CsOxide>, TopkGT>(
        "sketch_oxide CountSketch + size-k heap",
    ),
    scored::<polars::PolarsTopK, TopkGT>("polars exact: group_by(v).agg(len) sorted, top k"),
    // -------- Elastic (heavy-hitter; no query capability, throughput-only) --------
    plain::<elastic::ElasticLib>("asap_sketchlib::Elastic<DefaultXxHasher>"),
    plain::<elastic::ElasticOxide>("sketch_oxide::frequency::ElasticSketch"),
    // -------- Hydra (per-subpopulation statistics over labelled records) --------
    // One algorithm per cell type, because the cell decides which statistic the
    // grid answers and each is scored by a different comparator. See the module
    // header in `wrappers/hydra.rs`.
    scored::<hydra::HydraCms, SubpopFrequencyGT>(
        "asap_sketchlib::Hydra over Count-Min cells (subpopulation frequency)",
    ),
    scored::<polars::PolarsSubpopFrequency, SubpopFrequencyGT>(
        "polars exact: group_by(subset, v).agg(len) over every label subset",
    ),
    scored::<hydra::HydraHll, SubpopCardinalityGT>(
        "asap_sketchlib::Hydra over HyperLogLog cells (subpopulation cardinality)",
    ),
    scored::<polars::PolarsSubpopCardinality, SubpopCardinalityGT>(
        "polars exact: group_by(subset).agg(v.n_unique()) over every label subset",
    ),
    scored::<hydra::HydraKll, SubpopRankErrorGT>(
        "asap_sketchlib::Hydra over KLL cells (subpopulation quantile)",
    ),
    scored::<polars::PolarsSubpopQuantile, SubpopRankErrorGT>(
        "polars exact: sorted values per label subset, quantile by rank",
    ),
    // -------- Nitro / UnivMon (no query capability; throughput-only) --------
    plain::<nitro::NitroLib>("asap_sketchlib::NitroBatch<Vector2D<u32>>"),
    plain::<nitro::NitroOxide>("sketch_oxide::frequency::NitroSketch<CountMinSketch>"),
    plain::<univmon::UnivMonLib>("asap_sketchlib::UnivMon"),
    plain::<univmon::UnivMonOxide>("sketch_oxide::universal::UnivMon"),
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

/// Does `--accuracy` score this row? `None` if the row is unknown.
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

/// Resolve `(algorithm, impl)` to a concrete measurement and run it. The timed
/// half is always run; the accuracy half only when `acc.enabled`.
pub fn run(
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
    comparator: Option<&str>,
) -> Result<Vec<BenchReport>> {
    let row = find(algorithm, impl_name)
        .ok_or_else(|| anyhow::anyhow!("no impl '{impl_name}' for algorithm '{algorithm}'"))?;
    // Asked for a width this row's type cannot be built at — answerable from
    // the catalog, before a single item is generated.
    if width == Numeric::F64 && !row.picks_width {
        anyhow::bail!("{algorithm}/{impl_name} runs over i64 only; drop --dtype f64");
    }
    // A named comparator has to be one this row admits. Refused from the
    // catalog, by name, before anything is generated — the same rule the rest
    // of the selectors follow.
    let run = match comparator {
        None => row.run,
        Some(name) => row
            .comparators
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| *f)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{algorithm}/{impl_name} has no comparator '{name}'; it admits {}",
                    comparators_of(row)
                )
            })?,
    };
    Ok(run(cfg, spec, params, acc, width)?)
}

/// The comparator names a row admits, for an error message.
fn comparators_of(row: &Row) -> String {
    if row.comparators.is_empty() {
        return "none".to_string();
    }
    row.comparators
        .iter()
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Which comparators a row admits. `None` for an unknown row, so a frontend
/// can tell "no such row" from "that row is scored by nothing".
pub fn comparators(algorithm: &str, impl_name: &str) -> Option<Vec<&'static str>> {
    find(algorithm, impl_name).map(|row| row.comparators.iter().map(|(n, _)| *n).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{
        CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, HydraCmsParams,
        HydraHllParams, HydraKllParams, KllParams, NitroParams, SketchParams, TopkParams,
        UnivMonParams,
    };
    use std::collections::BTreeSet;

    /// One buildable config per family, from each params type's own
    /// `canonical()`, tagged with the row's own algorithm. The `panic!` arm is
    /// what makes a newly added family show up here rather than silently
    /// skipping the tests below.
    fn canonical_params(row: &Row) -> ParamSet {
        let a = row.algorithm;
        match row.family {
            "hll" => ParamSet::of_algorithm(a, &HllParams::canonical()),
            "kll" => ParamSet::of_algorithm(a, &KllParams::canonical()),
            "cms" => ParamSet::of_algorithm(a, &CmsParams::canonical()),
            "countsketch" => ParamSet::of_algorithm(a, &CountSketchParams::canonical()),
            "dd" => ParamSet::of_algorithm(a, &DdParams::canonical()),
            "elastic" => ParamSet::of_algorithm(a, &ElasticParams::canonical()),
            "nitro" => ParamSet::of_algorithm(a, &NitroParams::canonical()),
            "hydra-cms" => ParamSet::of_algorithm(a, &HydraCmsParams::canonical()),
            "hydra-hll" => ParamSet::of_algorithm(a, &HydraHllParams::canonical()),
            "hydra-kll" => ParamSet::of_algorithm(a, &HydraKllParams::canonical()),
            "topk" => ParamSet::of_algorithm(a, &TopkParams::canonical()),
            "univmon" => ParamSet::of_algorithm(a, &UnivMonParams::canonical()),
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
    /// type is not named here — each row materialises its own
    /// `Accumulator::Item`.
    fn smoke_spec() -> WorkloadSpec {
        WorkloadSpec::Generated(column(64, 256, 1))
    }

    fn column(cardinality: u64, size: usize, seed: u64) -> aqpbm_core::GenSpec {
        aqpbm_core::GenSpec {
            shape: aqpbm_core::Shape::Keys {
                cardinality,
                dist: aqpbm_core::Distribution::Uniform,
            },
            size,
            seed,
            string: None,
        }
    }

    /// The same, for the rows whose item is a record: two label columns and a
    /// value column. A row states which of the two it wants through
    /// `Row::takes_columns`, so neither is guessed here.
    fn smoke_columns_spec() -> WorkloadSpec {
        WorkloadSpec::Columns(vec![column(8, 256, 1), column(4, 256, 2), column(32, 256, 3)])
    }

    /// The spec shape `row` can actually ingest.
    fn spec_for(row: &Row) -> WorkloadSpec {
        if row.takes_columns {
            smoke_columns_spec()
        } else {
            smoke_spec()
        }
    }

    fn smoke_cfg() -> (BenchConfig, AccuracyCfg) {
        (
            BenchConfig {
                runs: 1,
                warmup_runs: 0,
                ..Default::default()
            },
            AccuracyCfg {
                enabled: false,
                max_probes: 0,
                record_query_calls: false,
            },
        )
    }

    /// Every row actually builds and ingests — the part the types cannot state.
    /// A row *is* its runner, so there is no `_` bail to fall into and no strings
    /// for the list and the dispatch to disagree about.
    #[test]
    fn every_catalog_entry_runs() {
        let (cfg, acc) = smoke_cfg();
        for r in ROWS {
            // Canonical, not `empty`: every family's params have required
            // fields, so `empty` builds nothing at all now that the exact
            // baselines parse their config too.
            let params = canonical_params(r);
            let got = run(
                r.algorithm,
                r.impl_name,
                &cfg,
                &spec_for(r),
                &params,
                &acc,
                Numeric::I64,
                None,
            );
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

    /// An algorithm's rows are only comparable if asked the same question, so they
    /// must agree on which configs are answerable — one row silently accepting a
    /// config its peers reject scores a different experiment.
    #[test]
    fn topk_rows_accept_and_reject_the_same_configs() {
        let spec = smoke_spec();
        let (cfg, acc) = smoke_cfg();
        for (cfg_spec, buildable) in [
            ("rows=5 cols=2048 k=5", true),
            ("rows=5 cols=2048 kk=5", false), // misspelled `k`
            ("rows=5 cols=2048 k=0", false),  // a top-k of nothing
            ("rows=5 cols=2048", false),      // no `k` at all
        ] {
            for r in ROWS.iter().filter(|r| r.family == "topk") {
                let params = config_point(r.algorithm, cfg_spec).unwrap();
                let got = run(
                    r.algorithm,
                    r.impl_name,
                    &cfg,
                    &spec,
                    &params,
                    &acc,
                    Numeric::I64,
                    None,
                );
                assert_eq!(
                    got.is_ok(),
                    buildable,
                    "topk/{} disagrees with the algorithm on `{cfg_spec}`: {got:?}",
                    r.impl_name
                );
            }
        }
    }
}
