//! Running **one cell** — one `(impl, config)` measured against one workload.
//!
//! A cell splits cleanly in two, because only one half needs the ground truth:
//!   - [`run_cell`] — the **timed** measurements (throughput / latency / CPU /
//!     memory). Generic over the sketch, no ground truth, fully monomorphised so
//!     the wrapper's `update` inlines into the hot loop.
//!   - [`score_cell`] — the **accuracy** measurement. Untimed, so it is free to
//!     carry the track's ground-truth calculator without touching the hot path.
//!
//! The frontend picks the concrete type `S` (and, for accuracy, the ground-truth calculator
//! `G`) and calls these. There is no per-track driver.
//!
//! This module also owns the run-time plumbing the frontend hands in:
//! [`WorkloadSpec`] (where items come from), [`Items`] (the materialised
//! workload), the view-narrowing [`FromItems`], and the two "cannot run" reasons
//! [`DtypeMismatch`] / [`RunError`].

use anyhow::Result;
use crate::config::ParamSet;
use crate::accumulator::Accumulator;
use crate::memory_footprint::MemoryFootprint;
use crate::workload::{
    BytesWorkload, F64Workload, I64Workload, StringWorkload, Workload,
};
use aqpbm_datagen::{DType, GenSpec};

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
    File { path: String },
}

impl WorkloadSpec {
    /// Materialise the items at the requested `dtype`. The dtype is *checked*
    /// against the spec, not inferred.
    pub fn build(self, dtype: DType) -> Result<Items> {
        match (self, dtype) {
            (WorkloadSpec::Generated(spec), DType::F64) => F64Workload::generate(&spec)
                .map(Items::F64)
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (WorkloadSpec::Generated(spec), DType::Str) => StringWorkload::generate(&spec)
                .map(Items::Str)
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (WorkloadSpec::Generated(spec), _) => I64Workload::generate(&spec)
                .map(Items::I64)
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (WorkloadSpec::File { path }, DType::I64) => {
                I64Workload::load(std::path::Path::new(&path))
                    .map(Items::I64)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            (WorkloadSpec::File { path }, other) => Err(anyhow::anyhow!(
                "--input {path} is read as an i64 stream, but --dtype {} was requested; \
                 generate the workload instead (--spec / --workload) to benchmark {}",
                other.as_str(),
                other.as_str(),
            )),
        }
    }
}

/// The materialised workload, at whichever dtype was asked for.
pub enum Items {
    I64(I64Workload),
    F64(F64Workload),
    Str(StringWorkload),
}

impl Items {
    pub fn dtype(&self) -> DType {
        match self {
            Items::I64(_) => DType::I64,
            Items::F64(_) => DType::F64,
            Items::Str(_) => DType::Str,
        }
    }
}

/// A row was handed a workload whose item type it cannot ingest.
#[derive(Debug, Clone)]
pub struct DtypeMismatch {
    pub wanted: &'static [DType],
    pub got: DType,
}

impl std::fmt::Display for DtypeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let wanted: Vec<&str> = self.wanted.iter().map(DType::as_str).collect();
        write!(
            f,
            "consumes {} items, workload is {}",
            wanted.join(" or "),
            self.got.as_str()
        )
    }
}

/// Why a `(impl, config)` cell cannot run: a data type it does not ingest, or a
/// construction it cannot satisfy.
#[derive(Debug)]
pub enum RunError {
    Dtype(DtypeMismatch),
    Build(BuildError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Dtype(e) => e.fmt(f),
            RunError::Build(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RunError {}

impl From<DtypeMismatch> for RunError {
    fn from(e: DtypeMismatch) -> Self {
        RunError::Dtype(e)
    }
}

impl From<BuildError> for RunError {
    fn from(e: BuildError) -> Self {
        RunError::Build(e)
    }
}

// ---------- the view axis ----------

/// Narrow the runtime [`Items`] enum to the concrete workload a sketch's `Item`
/// type consumes. One impl per item type; runs once per cell, before any timed
/// loop.
pub trait FromItems: Sized + Clone {
    type Wk: Workload<Item = Self>;
    const ACCEPTS: &'static [DType];
    fn narrow(items: &Items) -> Result<Self::Wk, DtypeMismatch>;
}

impl FromItems for i64 {
    type Wk = I64Workload;
    const ACCEPTS: &'static [DType] = &[DType::I64];
    fn narrow(items: &Items) -> Result<Self::Wk, DtypeMismatch> {
        match items {
            Items::I64(wk) => Ok(wk.clone()),
            other => Err(DtypeMismatch {
                wanted: Self::ACCEPTS,
                got: other.dtype(),
            }),
        }
    }
}

impl FromItems for f64 {
    type Wk = F64Workload;
    const ACCEPTS: &'static [DType] = &[DType::F64];
    fn narrow(items: &Items) -> Result<Self::Wk, DtypeMismatch> {
        match items {
            Items::F64(wk) => Ok(wk.clone()),
            other => Err(DtypeMismatch {
                wanted: Self::ACCEPTS,
                got: other.dtype(),
            }),
        }
    }
}

impl FromItems for String {
    type Wk = StringWorkload;
    const ACCEPTS: &'static [DType] = &[DType::I64, DType::Str];
    fn narrow(items: &Items) -> Result<Self::Wk, DtypeMismatch> {
        match items {
            Items::I64(wk) => Ok(StringWorkload::from_i64(wk)),
            Items::Str(wk) => Ok(wk.clone()),
            other => Err(DtypeMismatch {
                wanted: Self::ACCEPTS,
                got: other.dtype(),
            }),
        }
    }
}

impl FromItems for Vec<u8> {
    type Wk = BytesWorkload;
    const ACCEPTS: &'static [DType] = &[DType::I64, DType::Str];
    fn narrow(items: &Items) -> Result<Self::Wk, DtypeMismatch> {
        let strings = <String as FromItems>::narrow(items)?;
        Ok(BytesWorkload::from_strings(&strings))
    }
}

// ---------- the hot-loop body + construction ----------

/// The hot-loop body.
///
/// Generic and `#[inline(always)]`, so it is instantiated in whichever crate
/// names the concrete `S` — the wrapper's `update` and this call site still
/// land in one codegen unit and LLVM still folds the update into the loop,
/// even though the wrappers now live a crate away. `BenchRunner::run_timed`,
/// which drives the loop, has always been across that boundary.
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
pub fn run_cell<S>(
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: FromItems,
{
    let wk = <S::Item as FromItems>::narrow(items)?;
    S::init(params)?; // probe: the cell fails here if it cannot build
    Ok(BenchRunner::new(cfg.clone(), &wk, S::FAMILY, S::IMPL)
        .run_timed::<S, _, _>(|| built::<S>(params), insert_body))
}

/// Run the **timed** half of a parallel-insert cell (workers from `cfg.threads`).
pub fn run_cell_parallel<S>(
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: FromItems,
{
    let wk = <S::Item as FromItems>::narrow(items)?;
    let workers = cfg.threads;
    S::build(params, workers)?; // probe
    Ok(
        BenchRunner::new(cfg.clone(), &wk, S::FAMILY, S::IMPL).run_timed::<S, _, _>(
            move || S::build(params, workers).expect("construction proven by the probe above"),
            insert_body,
        ),
    )
}

/// Run the **accuracy** half of a cell against ground truth `gt`. Untimed, so
/// the calculator is free to live here. The caller supplies the one for this
/// sketch's track.
pub fn score_cell<S, G>(
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    gt: &G,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: FromItems,
    G: GroundTruth<S>,
{
    let wk = <S::Item as FromItems>::narrow(items)?;
    S::init(params)?; // probe
    Ok(BenchRunner::new(cfg.clone(), &wk, S::FAMILY, S::IMPL)
        .run_accuracy::<S, _, G, _>(|| built::<S>(params), insert_body, gt))
}
