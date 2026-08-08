//! Running **one cell** — one `(impl, config)` measured against one workload.
//!
//! [`run_cell`] takes the timed half (throughput / latency / CPU / memory),
//! monomorphised so the wrapper's `update` inlines; [`score_cell`] takes the
//! untimed accuracy half. Plus [`WorkloadSpec`], [`BenchItem`], [`RunError`].

use crate::accumulator::Accumulator;
use crate::config::ParamSet;
use crate::memory_footprint::MemoryFootprint;
use crate::workload::{
    BytesWorkload, F64Workload, I64Workload, Labeled, LabeledWorkload, StringWorkload, Workload,
};
use anyhow::Result;
use aqpbm_datagen::{GenSpec, GenValue};

use crate::accuracy::GroundTruth;
use crate::init::{BenchImpl, BuildError, InitSketch};
use crate::runner::{BenchConfig, BenchReport, BenchRunner};

// ---------- accuracy settings the frontend fills in ----------

/// Accuracy knobs. Consumed only by [`score_cell`] / the ground-truth calculator — the timed
/// path never sees them.
#[derive(Debug, Clone, Copy)]
pub struct AccuracyCfg {
    pub enabled: bool,
    /// Cap on distinct keys probed by frequency comparators. `0` → no cap.
    pub max_probes: usize,
    /// Record per-call query samples (the legacy per-call CSV). Honoured by
    /// cardinality / quantile comparators.
    pub record_query_calls: bool,
}

// ---------- where items come from, and what they materialise to ----------

/// Where a benchmark's items come from: generated in-process, or loaded.
#[derive(Debug, Clone)]
pub enum WorkloadSpec {
    Generated(GenSpec),
    /// A multi-column stream: `n - 1` label columns then one value column, each
    /// an ordinary [`GenSpec`]. Only the row types whose `Item` is a record read
    /// this; every single-column row refuses it by name.
    Columns(Vec<GenSpec>),
    File { path: String },
}

impl WorkloadSpec {
    /// Materialise at the item type `T`, which the row's `Accumulator::Item`
    /// already names. There is nothing to agree on: the caller's type is the
    /// only thing that picks an encoding.
    pub fn build<T: BenchItem>(&self) -> Result<T::Wk> {
        T::materialise(self)
    }
}

/// Why a `(impl, config)` cell cannot run: a workload it cannot obtain, or a
/// construction it cannot satisfy.
#[derive(Debug)]
pub enum RunError {
    Workload(anyhow::Error),
    Build(BuildError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Workload(e) => e.fmt(f),
            RunError::Build(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RunError {}

impl From<BuildError> for RunError {
    fn from(e: BuildError) -> Self {
        RunError::Build(e)
    }
}

impl From<anyhow::Error> for RunError {
    fn from(e: anyhow::Error) -> Self {
        RunError::Workload(e)
    }
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the workload that carries
/// it, and how to build one from a [`WorkloadSpec`]. A row's
/// `Accumulator::Item` fixes the encoding before anything is generated.
pub trait BenchItem: Sized + Clone {
    type Wk: Workload<Item = Self>;

    /// Whether this item is materialised from a [`WorkloadSpec::Columns`] list
    /// instead of a single-column spec. A `const`, so a catalog can read which
    /// kind of workload a row wants off the row's type, without building one.
    const TAKES_COLUMNS: bool = false;

    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk>;
}

/// A single-column row was handed a column list. Refused by name: zipping the
/// columns down to one would run the measurement over a stream nobody asked for.
fn reject_columns<T>(item: &str) -> Result<T> {
    Err(anyhow::anyhow!(
        "--spec names a column list, but this row ingests a plain `{item}` stream; \
         give it a single-column spec, or pick a row whose item is a record"
    ))
}

impl BenchItem for i64 {
    type Wk = I64Workload;
    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk> {
        match spec {
            WorkloadSpec::Generated(g) => {
                I64Workload::generate(g).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadSpec::Columns(_) => reject_columns("i64"),
            WorkloadSpec::File { path } => {
                I64Workload::load(std::path::Path::new(path)).map_err(|e| anyhow::anyhow!("{}", e))
            }
        }
    }
}

impl BenchItem for f64 {
    type Wk = F64Workload;
    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk> {
        match spec {
            WorkloadSpec::Generated(g) => {
                F64Workload::generate(g).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadSpec::Columns(_) => reject_columns("f64"),
            // `.bin` is a raw i64 stream with no header; reading it as f64
            // would reinterpret the bytes, not convert them.
            WorkloadSpec::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a raw i64 stream; generate the workload \
                 instead to benchmark f64"
            )),
        }
    }
}

impl BenchItem for String {
    type Wk = StringWorkload;
    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk> {
        match spec {
            WorkloadSpec::Generated(g) => {
                StringWorkload::generate(g).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadSpec::Columns(_) => reject_columns("String"),
            // Decimal-formatted, the same rendering the i64-sourced path used.
            WorkloadSpec::File { path } => I64Workload::load(std::path::Path::new(path))
                .map(|wk| StringWorkload::from_i64(&wk))
                .map_err(|e| anyhow::anyhow!("{}", e)),
        }
    }
}

impl BenchItem for Vec<u8> {
    type Wk = BytesWorkload;
    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk> {
        Ok(BytesWorkload::from_strings(
            &<String as BenchItem>::materialise(spec)?,
        ))
    }
}

/// The record item: only a column list materialises one. A single-column spec
/// is refused instead of being padded into a one-label record, because the
/// column count is what a grouped sketch's cost is a function of.
impl<V: GenValue> BenchItem for Labeled<V> {
    type Wk = LabeledWorkload<V>;
    const TAKES_COLUMNS: bool = true;
    fn materialise(spec: &WorkloadSpec) -> Result<Self::Wk> {
        match spec {
            WorkloadSpec::Columns(cols) => {
                LabeledWorkload::generate(cols).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadSpec::Generated(_) => Err(anyhow::anyhow!(
                "this row ingests labelled records, so it needs a column list: \
                 pass `--spec` a JSON array of column specs, the last one being \
                 the value column"
            )),
            WorkloadSpec::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a single-column stream; this row ingests \
                 labelled records, so it needs a `--spec` column list"
            )),
        }
    }
}

// ---------- the hot-loop body + construction ----------

/// The hot-loop body. Generic and `#[inline(always)]`, so it instantiates in
/// whichever crate names the concrete `S`: the wrapper's `update` and this call
/// site land in one codegen unit, and LLVM folds the update into the loop.
#[inline(always)]
pub fn insert_body<S: Accumulator>(s: &mut S, it: &S::Item) {
    s.update(it);
}

/// Construct a fresh sketch. Construction is proven by the probe at the top of
/// each cell function, so the per-run factory unwraps.
fn built<S: InitSketch>(params: &ParamSet) -> S {
    S::init(params).expect("construction proven by the probe in the cell function")
}

// ---------- parallel-insert construction ----------

/// A parallel-insert wrapper: its constructor also takes the worker count (a
/// run knob, not a sketch parameter), so it cannot be an [`InitSketch`].
pub trait ParallelInit: Accumulator + Sized {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError>;
}

// ---------- running one cell ----------

/// Run the **timed** half of a cell: throughput / latency / CPU / memory. No
/// ground truth — the ground-truth calculator never touches the hot path.
pub fn run_cell<S, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    gt: Option<&G>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruth<S>,
{
    let wk = <S::Item as BenchItem>::materialise(spec)?;
    S::init(params)?; // probe: the cell fails here if it cannot build
    Ok(BenchRunner::new(cfg.clone(), &wk, S::ALGORITHM, S::IMPL)
        .run::<S, _, G, _>(|| built::<S>(params), insert_body, gt))
}

/// Run the **timed** half of a parallel-insert cell (workers from `cfg.threads`).
pub fn run_cell_parallel<S, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    gt: Option<&G>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruth<S>,
{
    let wk = <S::Item as BenchItem>::materialise(spec)?;
    let workers = cfg.threads;
    S::build(params, workers)?; // probe
    Ok(
        BenchRunner::new(cfg.clone(), &wk, S::ALGORITHM, S::IMPL).run::<S, _, G, _>(
            move || S::build(params, workers).expect("construction proven by the probe above"),
            insert_body,
            gt,
        ),
    )
}

