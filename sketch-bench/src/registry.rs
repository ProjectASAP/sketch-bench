//! The registry: one table naming every `(algorithm, impl)` this crate exposes,
//! and the dispatch resolving one to a concrete sketch type. It lives here,
//! not in the CLI, so a future `aqp-bench` can ship its own. A row is a
//! *type*, not a pair of strings — algorithm, impl name and `scores_accuracy`
//! are projected off it, so the list and the code cannot drift apart.

use anyhow::Result;

use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;

use crate::params::ParamSet;
use crate::wrappers::{cms, cs, hll, hydra, kll};

pub use aqpbm_core::request::{Capability, Numeric};

/// Who a row is, as data. Was read off the row's type through a `BenchImpl`
/// trait whose only content was these strings; it is stated here instead,
/// because the registry is what owns them.
#[derive(Clone, Copy)]
pub struct RowIdentity {
    pub family: &'static str,
    pub algorithm: &'static str,
    pub impl_name: &'static str,
    /// Does this row's `SketchOps` supply a `merge`? Stated here because the
    /// `Option` inside the thunk is not readable in `const` context.
    pub supports_merge: bool,
    /// Likewise for `prepare` — the doc's `prepare_for_query` axis.
    pub supports_prepare: bool,
}

/// A row's runner: given the loop knobs, the request and the produced data, run
/// every selected square and report.
///
/// Every arm of the dispatch this sits behind is monomorphic, so the row runs
/// its own measurements with the sketch type known: nothing is erased, and what
/// crosses to a frontend is records rather than closures.
type RunFn = fn(&MeasureConfig, &Requirement, WorkloadData) -> Result<Vec<BenchReport>, RunError>;

/// One registry entry. Built only by the constructors below, so `family`,
/// `algorithm`, `impl_name` and `scores_accuracy` are always projections of the
/// row's type and its runner — never hand-written strings that could drift from
/// it.
pub struct SketchId {
    pub family: &'static str,
    pub algorithm: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
}

// ---------- how a row builds its ground truth ----------

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* in [`REGISTRY`] and the row stays `const`.
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

// ---------- the registry ----------

pub const REGISTRY: &[SketchId] = &[
    SketchId {
        family: "cms",
        algorithm: "cms-fixedmatrix-fastpath",
        impl_name: "asap_sketchlib",
        description: "CountMin sketch from asap_sketchlib based on Fixed Size Matrix, FastPath",
    },
    SketchId {
        family: "hll",
        algorithm: "hll",
        impl_name: "asap_sketchlib",
        description: "HyperLogLog from asap_sketchlib",
    },
];
// ---------- what the frontend asks ----------

fn find(algorithm: &str, impl_name: &str) -> Option<&'static SketchId> {
    REGISTRY
        .iter()
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
    let algo_w = REGISTRY
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = REGISTRY
        .iter()
        .map(|r| r.impl_name.len())
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(REGISTRY.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  description",
        "# algorithm", "impl"
    ));
    let mut current: Option<&str> = None;
    for r in REGISTRY {
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
    REGISTRY.iter().any(|r| r.algorithm == algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field. `None`
/// if the algorithm is unknown, which the frontend has already ruled out by the
/// time it asks.
pub fn family_of(algorithm: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|r| r.algorithm == algorithm)
        .map(|r| r.family)
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
    UnknownImpl {
        algorithm: String,
        impl_name: String,
    },
    /// A width the row's item type cannot be built at.
    WidthUnsupported {
        algorithm: String,
        impl_name: String,
    },
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
            ResolveError::UnknownImpl {
                algorithm,
                impl_name,
            } => {
                write!(f, "no impl '{impl_name}' for algorithm '{algorithm}'")
            }
            ResolveError::WidthUnsupported {
                algorithm,
                impl_name,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} runs over i64 only; drop --dtype f64"
                )
            }
            ResolveError::OperationUnsupported {
                algorithm,
                impl_name,
                operation,
                admits,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} has no {operation}; it can be measured over {admits}"
                )
            }
            ResolveError::MetricUnsupported {
                algorithm,
                impl_name,
                metric,
                capability,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} answers no statistic (capability {capability}), \
                     so nothing can score its {metric}"
                )
            }
            ResolveError::NothingMeasuresIt { operation, metric } => {
                write!(
                    f,
                    "nothing measures the {metric} of {operation}, for any sketch"
                )
            }
            ResolveError::UnknownComparator {
                algorithm,
                impl_name,
                name,
                admits,
            } => {
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
/// closures in this design are the per-row `ask` bodies written in [`REGISTRY`];
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
    /// Runs this request. Captures the row's monomorphic runner and the request
    /// it resolved, so a caller supplies only the loop knobs and the data it
    /// generated. The params, the width, the workers and the shards are all in
    /// that request, and are not passed a second time.
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
