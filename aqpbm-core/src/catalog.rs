//! What a benchmarkable row *is*, and how one runs: the [`Row`] type, the
//! runners behind it, the constructors that build one, and the lookups a
//! frontend puts to a table of them.
//!
//! No knowledge of which sketches exist lives here. A **bundle** is a crate
//! that owns wrappers and publishes a `&'static [Row]` naming them;
//! `sketch-bench` is one such bundle and not the only one it is possible to
//! write. Every lookup below therefore takes the table as an argument instead
//! of reading a global, which is the whole of what makes a second bundle
//! possible: it depends on this crate, not on another bundle's sketches.
//!
//! A row is one `(algorithm, impl, ground truth)`, and every field is projected
//! off its types. Scoring one sketch against a second ground truth is a second
//! row, not a second entry inside the first.

use anyhow::Result;

use crate::accumulator::Accumulator;
use crate::accuracy::cardinality::CardinalityGT;
use crate::accuracy::frequency::FrequencyGT;
use crate::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use crate::accuracy::subpopulation::{SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT};
use crate::accuracy::topk::TopkGT;
use crate::accuracy::GroundTruth;
use crate::cell::{self, BenchItem, ParallelInit, RunError, WorkloadSpec};
use crate::config::ParamSet;
use crate::init::{BenchImpl, InitSketch};
use crate::memory_footprint::MemoryFootprint;
use crate::runner::{needs_ground_truth, BenchConfig, BenchReport, NoGT};

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
pub type RunFn =
    fn(&BenchConfig, &WorkloadSpec, &ParamSet, Numeric) -> Result<Vec<BenchReport>, RunError>;

/// One catalog entry.
///
/// Prefer the constructors below: they read every field off the row's types, so
/// none can drift from what actually runs. The fields are open because a bundle
/// whose row type is selected by a construction parameter has to supply a `run`
/// this crate cannot name.
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
    /// The one field that is genuinely new data, and so is written at the row's
    /// registration site.
    pub description: &'static str,
    /// What scores this row, by the name `--ground-truth` selects it with, and
    /// the third of its identity. `None` for a row that answers no query.
    ///
    /// Derived from the calculator type, which the compiler has already checked
    /// the row's capabilities can satisfy — so the table cannot name a scoring
    /// the row could not answer.
    pub ground_truth: Option<&'static str>,
    /// Can this row run at [`Numeric::F64`]? Derived: only `ordered` rows can.
    pub picks_width: bool,
    /// Does this row ingest labelled records, and so need a `--spec` column
    /// list instead of a single-column spec? Derived off the row's item type.
    pub takes_columns: bool,
    /// What this row does when selected. One `fn` pointer to a monomorphised
    /// runner, resolved once per invocation.
    pub run: RunFn,
}

impl Row {
    /// Can anything score this row?
    pub fn scores_accuracy(&self) -> bool {
        self.ground_truth.is_some()
    }
}

// ---------- how a row builds its ground truth ----------

/// What a ground truth is called. Separate from [`GroundTruthCalculator`] and
/// not generic over `S`, so a name can be read without naming a sketch type —
/// which a dispatching row, whose sketch type is chosen at run time, needs.
pub trait GroundTruthName {
    const NAME: &'static str;
}

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* where the row is registered, and the row stays `const`.
pub trait GroundTruthCalculator<S: Accumulator>: GroundTruth<S> + GroundTruthName {
    fn build(params: &ParamSet) -> Self;
}

macro_rules! ground_truth_name {
    ($ty:ty, $name:literal) => {
        impl GroundTruthName for $ty {
            const NAME: &'static str = $name;
        }
    };
}

ground_truth_name!(CardinalityGT, "cardinality");
ground_truth_name!(FrequencyGT, "frequency");
ground_truth_name!(SubpopFrequencyGT, "subpop-frequency");
ground_truth_name!(SubpopCardinalityGT, "subpop-cardinality");
ground_truth_name!(SubpopRankErrorGT, "subpop-rank-error");
ground_truth_name!(RankErrorGT, "rank-error");
ground_truth_name!(RelativeErrorGT, "relative-error");
ground_truth_name!(TopkGT, "topk");

impl<S: Accumulator> GroundTruthCalculator<S> for CardinalityGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        CardinalityGT
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for FrequencyGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        FrequencyGT
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopFrequencyGT
where
    Self: GroundTruth<S>,
{
    /// Column 0. A second column, or a deeper subset, is a second ground truth
    /// type and so a second row.
    fn build(_params: &ParamSet) -> Self {
        SubpopFrequencyGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopCardinalityGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        SubpopCardinalityGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopRankErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        SubpopRankErrorGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RankErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        RankErrorGT {}
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RelativeErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        RelativeErrorGT {}
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for TopkGT
where
    Self: GroundTruth<S>,
{
    /// Scores against the same `k` the sketch was built with — a different prefix
    /// would measure the mismatch, not the sketch. Infallible because the timed
    /// half runs first, so an unreadable `k` has already failed the build.
    fn build(params: &ParamSet) -> Self {
        TopkGT {
            k: params
                .field::<usize>("k")
                .expect("the row built, so its params parse and carry `k`"),
        }
    }
}

// ---------- the ways a row runs ----------

/// Every square the request selects, scored against `G` where one needs it.
pub fn run_scored<S, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    // A ground truth is built when the request reaches a square that cannot run
    // without one. Nobody has to ask for it: needing one is a property of the
    // squares selected, not a separate decision.
    let gt = needs_ground_truth(cfg.operations, cfg.metrics).then(|| G::build(params));
    let mut reports = cell::run_cell::<S, G>(cfg, spec, params, gt.as_ref())?;
    // Stamped from the type that ran, not looked up from the request, so the
    // record cannot name a ground truth other than the one that scored it.
    for report in &mut reports {
        report.ground_truth = Some(G::NAME.to_string());
    }
    Ok(reports)
}

/// An ordered quantile algorithm (KLL, DDSketch): the row names both widths and
/// the caller's [`Numeric`] picks one. The only place a runtime value still
/// selects an item type.
pub fn run_ordered<Si, Sf, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    match width {
        Numeric::I64 => run_scored::<Si, G>(cfg, spec, params, width),
        Numeric::F64 => run_scored::<Sf, G>(cfg, spec, params, width),
    }
}

/// A row with no query capability: timed only, no ground truth, nothing to score.
pub fn run_plain<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell::<S, NoGT>(cfg, spec, params, None)
}

/// A parallel-insert row: built with the worker count, so not an `InitSketch`.
pub fn run_parallel<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell_parallel::<S, NoGT>(cfg, spec, params, None)
}

// ---------- the row constructors ----------
// Each reads every field off the row's types. `const fn`, so a bundle's table
// stays `const`. The bounds are what reject a bad row, and they are checked at
// the registration site whether or not that table is `const`.

pub const fn scored<S, G>(description: &'static str) -> Row
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
        ground_truth: Some(G::NAME),
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_scored::<S, G>,
    }
}

pub const fn ordered<Si, Sf, G>(description: &'static str) -> Row
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
        ground_truth: Some(G::NAME),
        picks_width: true,
        takes_columns: <Si::Item as BenchItem>::TAKES_COLUMNS,
        run: run_ordered::<Si, Sf, G>,
    }
}

pub const fn plain<S>(description: &'static str) -> Row
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        // Answers no query, so nothing scores it.
        ground_truth: None,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_plain::<S>,
    }
}

pub const fn parallel_row<S>(description: &'static str) -> Row
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        family: S::FAMILY,
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        ground_truth: None,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_parallel::<S>,
    }
}

// ---------- what the frontend asks ----------

/// The row a request names. `ground_truth: None` takes the first row for the
/// `(algorithm, impl)` pair, which is the only one unless a sketch is registered
/// against several.
fn find(
    rows: &'static [Row],
    algorithm: &str,
    impl_name: &str,
    ground_truth: Option<&str>,
) -> Option<&'static Row> {
    rows.iter().find(|r| {
        r.algorithm == algorithm
            && r.impl_name == impl_name
            && match ground_truth {
                None => true,
                Some(g) => r.ground_truth == Some(g),
            }
    })
}

/// One line per row, grouped by family with a blank line between groups, since
/// the family is what a reader picks from before they pick a variant. Rows keep
/// declaration order inside a family.
///
/// Columns are sized to the longest name present, so adding a longer variant
/// widens the table instead of breaking its alignment. The first line is the
/// header, so a caller prints exactly what this returns.
pub fn list(rows: &'static [Row]) -> Vec<String> {
    const GT_HEADER: &str = "ground-truth";
    let algo_w = rows
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = rows.iter().map(|r| r.impl_name.len()).max().unwrap_or(0);
    let gt_w = rows
        .iter()
        .map(|r| r.ground_truth.unwrap_or("-").len())
        .max()
        .unwrap_or(0)
        .max(GT_HEADER.len());
    let mut out = Vec::with_capacity(rows.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  {:gt_w$}  description",
        "# algorithm", "impl", GT_HEADER
    ));
    let mut current: Option<&str> = None;
    for r in rows {
        if current != Some(r.family) {
            out.push(String::new());
            current = Some(r.family);
        }
        out.push(format!(
            "{:algo_w$}  {:impl_w$}  {:gt_w$}  {}",
            r.algorithm,
            r.impl_name,
            r.ground_truth.unwrap_or("-"),
            r.description
        ));
    }
    out
}

pub fn algorithm_exists(rows: &'static [Row], algorithm: &str) -> bool {
    rows.iter().any(|r| r.algorithm == algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field. `None`
/// if the algorithm is unknown, which the frontend has already ruled out by the
/// time it asks.
pub fn family_of(rows: &'static [Row], algorithm: &str) -> Option<&'static str> {
    rows.iter()
        .find(|r| r.algorithm == algorithm)
        .map(|r| r.family)
}

/// Can a ground truth score this `(algorithm, impl)`? `None` if it is unknown.
pub fn scores_accuracy(rows: &'static [Row], algorithm: &str, impl_name: &str) -> Option<bool> {
    find(rows, algorithm, impl_name, None).map(|r| r.scores_accuracy())
}

/// Parse the single `--config` point for an algorithm, checking the algorithm exists.
pub fn config_point(rows: &'static [Row], algorithm: &str, spec: &str) -> Result<ParamSet> {
    if !algorithm_exists(rows, algorithm) {
        anyhow::bail!("unknown sketch algorithm: {algorithm}");
    }
    ParamSet::single(algorithm, spec).map_err(Into::into)
}

/// Resolve `(algorithm, impl)` to a concrete measurement and run it. The timed
/// half is always run; the accuracy half only when `acc.enabled`.
pub fn run(
    rows: &'static [Row],
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
    ground_truth: Option<&str>,
) -> Result<Vec<BenchReport>> {
    // Refused from the catalog, by name, before anything is generated — the
    // same rule the rest of the selectors follow.
    let row = find(rows, algorithm, impl_name, ground_truth).ok_or_else(|| match ground_truth {
        None => anyhow::anyhow!("no impl '{impl_name}' for algorithm '{algorithm}'"),
        Some(name) => anyhow::anyhow!(
            "{algorithm}/{impl_name} has no ground truth '{name}'; it is registered against {}",
            named(&ground_truths(rows, algorithm, impl_name))
        ),
    })?;
    // Asked for a width this row's type cannot be built at — answerable from
    // the catalog, before a single item is generated.
    if width == Numeric::F64 && !row.picks_width {
        anyhow::bail!("{algorithm}/{impl_name} runs over i64 only; drop --dtype f64");
    }
    Ok((row.run)(cfg, spec, params, width)?)
}

/// Every ground truth an `(algorithm, impl)` is registered against — one per
/// row. Empty when the pair is unknown or answers no query.
pub fn ground_truths(
    rows: &'static [Row],
    algorithm: &str,
    impl_name: &str,
) -> Vec<&'static str> {
    rows.iter()
        .filter(|r| r.algorithm == algorithm && r.impl_name == impl_name)
        .filter_map(|r| r.ground_truth)
        .collect()
}

fn named(names: &[&str]) -> String {
    if names.is_empty() {
        return "none".to_string();
    }
    names.join(", ")
}
