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
//! A row is a *type*, not a pair of strings. Algorithm, impl name and
//! `scores_accuracy` are projected off it, so the list and the code cannot
//! drift apart. `run` is a plain `fn` pointer to a monomorphised
//! `run_scored::<S, G>`, resolved once per invocation, so nothing here is on a
//! timed path.

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

/// The comparators a row can be scored by, keyed by the name `--comparator`
/// selects them with.
pub type Comparators = &'static [(&'static str, RunFn)];

/// One catalog entry.
///
/// Prefer the constructors below: they read `family`, `algorithm` and
/// `impl_name` off the row's type and fix `scores_accuracy`, so those cannot
/// drift from what actually runs. The fields are open because a bundle whose
/// row type is selected by a construction parameter has to supply a `run` this
/// crate cannot name, and it still projects its identity off its own types.
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
    /// Can a comparator score this row? Derived: true iff it was built with a
    /// constructor that takes a ground-truth calculator.
    pub scores_accuracy: bool,
    /// Can this row run at [`Numeric::F64`]? Derived: only `ordered` rows can.
    pub picks_width: bool,
    /// Does this row ingest labelled records, and so need a `--spec` column
    /// list instead of a single-column spec? Derived off the row's item type.
    pub takes_columns: bool,
    /// What this row does when selected. One `fn` pointer to a monomorphised
    /// runner, resolved once per invocation.
    pub run: RunFn,
    /// The comparators this row admits, by name, first one the default. Every
    /// entry is checked by the compiler: a calculator the row's capabilities
    /// cannot satisfy will not build, so the table cannot offer a comparison
    /// the row could not answer.
    ///
    /// A list, so how many comparators a row admits is a property of the row
    /// and not of which constructor built it. Two is written as two entries.
    pub comparators: Comparators,
}

// ---------- how a row builds its ground truth ----------

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* where the row is registered, and the row stays `const`.
pub trait GroundTruthCalculator<S: Accumulator>: GroundTruth<S> {
    /// The name `--comparator` selects this one by. One capability can carry
    /// several comparators, and this is what tells them apart on the command
    /// line.
    const NAME: &'static str;
    fn build(params: &ParamSet) -> Self;
}

impl<S: Accumulator> GroundTruthCalculator<S> for CardinalityGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "cardinality";
    fn build(_params: &ParamSet) -> Self {
        CardinalityGT
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for FrequencyGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "frequency";
    fn build(_params: &ParamSet) -> Self {
        FrequencyGT
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
    fn build(_params: &ParamSet) -> Self {
        SubpopFrequencyGT {
            label_column: 0,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopCardinalityGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "subpop-cardinality";
    /// Column 0, for the same reason as [`SubpopFrequencyGT`].
    fn build(_params: &ParamSet) -> Self {
        SubpopCardinalityGT {
            label_column: 0,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopRankErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "subpop-rank-error";
    /// Column 0, for the same reason as [`SubpopFrequencyGT`]. The most
    /// expensive comparator in the catalog: 101 estimate calls per group.
    fn build(_params: &ParamSet) -> Self {
        SubpopRankErrorGT {
            label_column: 0,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RankErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "rank-error";
    fn build(_params: &ParamSet) -> Self {
        RankErrorGT {
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RelativeErrorGT
where
    Self: GroundTruth<S>,
{
    const NAME: &'static str = "relative-error";
    fn build(_params: &ParamSet) -> Self {
        RelativeErrorGT {
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
    // A comparator is built when the request reaches a square that cannot run
    // without one. Nobody has to ask for it: needing one is a property of the
    // squares selected, not a separate decision.
    let gt = needs_ground_truth(cfg.operations, cfg.metrics).then(|| G::build(params));
    Ok(cell::run_cell::<S, G>(cfg, spec, params, gt.as_ref())?)
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
// Each reads `S::FAMILY` / `S::ALGORITHM` / `S::IMPL` off the type and fixes
// `scores_accuracy`. `const fn`, so a bundle's table stays `const`. The bounds
// are what reject a bad row, and they are checked at the registration site
// whether or not that table is `const`.

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
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_scored::<S, G>,
        comparators: &[(G::NAME, run_scored::<S, G>)],
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
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_plain::<S>,
        // Answers no query, so nothing scores it.
        comparators: &[],
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
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_parallel::<S>,
        comparators: &[],
    }
}

// ---------- what the frontend asks ----------

fn find(rows: &'static [Row], algorithm: &str, impl_name: &str) -> Option<&'static Row> {
    rows.iter()
        .find(|r| r.algorithm == algorithm && r.impl_name == impl_name)
}

/// One line per row, grouped by family with a blank line between groups, since
/// the family is what a reader picks from before they pick a variant. Rows keep
/// declaration order inside a family.
///
/// The algorithm column is sized to the longest name present, so adding a
/// longer variant widens the table instead of breaking its alignment. The first
/// line is the header, so a caller prints exactly what this returns.
pub fn list(rows: &'static [Row]) -> Vec<String> {
    let algo_w = rows
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = rows.iter().map(|r| r.impl_name.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(rows.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  description",
        "# algorithm", "impl"
    ));
    let mut current: Option<&str> = None;
    for r in rows {
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

/// Can a comparator score this row? `None` if the row is unknown.
pub fn scores_accuracy(rows: &'static [Row], algorithm: &str, impl_name: &str) -> Option<bool> {
    find(rows, algorithm, impl_name).map(|r| r.scores_accuracy)
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
    comparator: Option<&str>,
) -> Result<Vec<BenchReport>> {
    let row = find(rows, algorithm, impl_name)
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
    Ok(run(cfg, spec, params, width)?)
}

/// The comparator names a row admits, for an error message.
pub fn comparators_of(row: &Row) -> String {
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
pub fn comparators(
    rows: &'static [Row],
    algorithm: &str,
    impl_name: &str,
) -> Option<Vec<&'static str>> {
    find(rows, algorithm, impl_name).map(|row| row.comparators.iter().map(|(n, _)| *n).collect())
}
