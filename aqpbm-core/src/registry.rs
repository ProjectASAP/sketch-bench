//! What a [`Registration`] is, how one runs, and the lookups a frontend puts to a
//! table of them. Knows no sketch — a bundle publishes its own `REGISTRY`, which
//! is why every lookup takes it as an argument. One registration is one
//! `(algorithm, impl, ground truth)`; registering a sketch again is how it gets a
//! second ground truth.

use anyhow::Result;

use crate::accumulator::Accumulator;
use crate::accuracy::GroundTruthCalculator;
use crate::cell::{self, BenchItem, ParallelInit, RunError, WorkloadSpec};
use crate::config::ParamSet;
use crate::init::{BenchImpl, InitSketch};
use crate::memory_footprint::MemoryFootprint;
use crate::runner::{needs_ground_truth, BenchConfig, BenchReport, NoGT};

// ---------- what a registration is ----------

/// Only an `ordered` registration (KLL, DDSketch) builds at either width; every
/// other item type is fixed by its Rust type, so a bad width is refused before
/// anything is generated.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Numeric {
    #[default]
    I64,
    F64,
}

/// The executable half: everything the frontend can hand a cell.
pub type RunFn =
    fn(&BenchConfig, &WorkloadSpec, &ParamSet, Numeric) -> Result<Vec<BenchReport>, RunError>;

/// One registered measurement. Prefer the constructors below, which read every
/// field off its types; the fields are open because a bundle whose sketch type
/// is chosen by a construction parameter supplies a `run` this crate cannot
/// name.
pub struct Registration {
    /// What a cross-library comparison groups by: one parameter vocabulary is
    /// one family.
    pub family: &'static str,
    /// Structural variant included; `--algorithm` matches it exactly.
    pub algorithm: &'static str,
    /// The implementing library, and only that.
    pub impl_name: &'static str,
    /// The one field that is genuinely new data at the registration site.
    pub description: &'static str,
    /// The third of the identity, and what `--ground-truth` names. `None` when
    /// nothing scores it.
    pub ground_truth: Option<&'static str>,
    /// Can this run at [`Numeric::F64`]? Only `ordered` ones can.
    pub picks_width: bool,
    /// Does this want a `--spec` column list rather than a single column?
    pub takes_columns: bool,
    pub run: RunFn,
}

impl Registration {
    pub fn scores_accuracy(&self) -> bool {
        self.ground_truth.is_some()
    }
}

// ---------- the ways one runs ----------

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
    // Built only when the selected squares cannot run without one; nobody has
    // to ask.
    let gt = needs_ground_truth(cfg.operations, cfg.metrics).then(|| G::build(params));
    let mut reports = cell::run_cell::<S, G>(cfg, spec, params, gt.as_ref())?;
    // From the type that ran, not the request, so the record cannot name a
    // ground truth other than the one that scored it.
    for report in &mut reports {
        report.ground_truth = Some(G::NAME.to_string());
    }
    Ok(reports)
}

/// An ordered quantile algorithm (KLL, DDSketch): both widths are named and the
/// caller's [`Numeric`] picks one. The only place a runtime value still selects
/// an item type.
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

/// No query capability: timed only, nothing to score.
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

/// Parallel insert: built with the worker count, so not an `InitSketch`.
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

// ---------- the constructors ----------
// Every field read off the registration's types. `const fn`, so a bundle's
// table stays `const`; the bounds are what reject a bad one at its site.

pub const fn scored<S, G>(description: &'static str) -> Registration
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    Registration {
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

pub const fn ordered<Si, Sf, G>(description: &'static str) -> Registration
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    Registration {
        // Both halves are one registration; the i64 one names it.
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

pub const fn plain<S>(description: &'static str) -> Registration
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Registration {
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

pub const fn parallel_row<S>(description: &'static str) -> Registration
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Registration {
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

/// `ground_truth: None` takes the first registration for the pair, which is the
/// only one unless a sketch is registered against several.
fn find(
    registry: &'static [Registration],
    algorithm: &str,
    impl_name: &str,
    ground_truth: Option<&str>,
) -> Option<&'static Registration> {
    registry.iter().find(|r| {
        r.algorithm == algorithm
            && r.impl_name == impl_name
            && match ground_truth {
                None => true,
                Some(g) => r.ground_truth == Some(g),
            }
    })
}

/// One line each, grouped by family, declaration order within it. Columns size
/// to the longest name present; the first line is the header, so a caller prints
/// exactly what this returns.
pub fn list(registry: &'static [Registration]) -> Vec<String> {
    const GT_HEADER: &str = "ground-truth";
    let algo_w = registry
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = registry.iter().map(|r| r.impl_name.len()).max().unwrap_or(0);
    let gt_w = registry
        .iter()
        .map(|r| r.ground_truth.unwrap_or("-").len())
        .max()
        .unwrap_or(0)
        .max(GT_HEADER.len());
    let mut out = Vec::with_capacity(registry.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  {:gt_w$}  description",
        "# algorithm", "impl", GT_HEADER
    ));
    let mut current: Option<&str> = None;
    for r in registry {
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

pub fn algorithm_exists(registry: &'static [Registration], algorithm: &str) -> bool {
    registry.iter().any(|r| r.algorithm == algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field.
pub fn family_of(registry: &'static [Registration], algorithm: &str) -> Option<&'static str> {
    registry.iter()
        .find(|r| r.algorithm == algorithm)
        .map(|r| r.family)
}

/// Can a ground truth score this `(algorithm, impl)`? `None` if it is unknown.
pub fn scores_accuracy(registry: &'static [Registration], algorithm: &str, impl_name: &str) -> Option<bool> {
    find(registry, algorithm, impl_name, None).map(|r| r.scores_accuracy())
}

/// Parse the single `--config` point for an algorithm, checking the algorithm exists.
pub fn config_point(registry: &'static [Registration], algorithm: &str, spec: &str) -> Result<ParamSet> {
    if !algorithm_exists(registry, algorithm) {
        anyhow::bail!("unknown sketch algorithm: {algorithm}");
    }
    ParamSet::single(algorithm, spec).map_err(Into::into)
}

/// Resolve a registration and run it.
pub fn run(
    registry: &'static [Registration],
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
    ground_truth: Option<&str>,
) -> Result<Vec<BenchReport>> {
    // Refused by name before anything is generated.
    let row = find(registry, algorithm, impl_name, ground_truth).ok_or_else(|| match ground_truth {
        None => anyhow::anyhow!("no impl '{impl_name}' for algorithm '{algorithm}'"),
        Some(name) => anyhow::anyhow!(
            "{algorithm}/{impl_name} has no ground truth '{name}'; it is registered against {}",
            named(&ground_truths(registry, algorithm, impl_name))
        ),
    })?;
    // A width this row's type cannot be built at.
    if width == Numeric::F64 && !row.picks_width {
        anyhow::bail!("{algorithm}/{impl_name} runs over i64 only; drop --dtype f64");
    }
    Ok((row.run)(cfg, spec, params, width)?)
}

/// Empty when the pair is unknown or answers no query.
pub fn ground_truths(
    registry: &'static [Registration],
    algorithm: &str,
    impl_name: &str,
) -> Vec<&'static str> {
    registry.iter()
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
