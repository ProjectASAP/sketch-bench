//! Running **one cell** — one `(impl, config)` measured against one workload.
//!
//! [`run_cell`] runs one square of the grid, monomorphised so the wrapper's
//! `update` inlines. Plus [`WorkloadSpec`], [`BenchItem`], [`RunError`].

use crate::ops::SketchOps;
use crate::config::ParamSet;
use crate::memory_footprint::MemoryFootprint;
use crate::workload::{
    BytesWorkload, F64Workload, I64Workload, Labeled, LabeledWorkload, StringWorkload, Workload,
};
use anyhow::Result;
use aqpbm_datagen::{ColumnItem, GeneratedTable, TableDescription};

use crate::accuracy::GroundTruth;
use crate::init::{BuildError, InitSketch};
use crate::runner::{BenchConfig, BenchReport, BenchRunner};

// ---------- where items come from, and what they materialise to ----------

/// Where a benchmark's items come from: generated in-process, or loaded.
///
/// One [`TableDescription`] covers both the single-column stream a plain row
/// ingests and the column list a record-ingesting row needs, so there is no
/// variant per column count — the column count is what a row checks.
///
/// The two generated variants differ in *who wrote the `data_type`*. A spec file
/// states it, and stands as written: an option that edited a field of the user's
/// file would make the file a suggestion. The inline options state a
/// distribution and a size and no type at all, so the row's item type is what
/// fills it in.
#[derive(Debug, Clone)]
pub enum WorkloadSpec {
    Generated(TableDescription),
    Inline(TableDescription),
    File { path: String },
}

/// A workload that has already been produced, on its way to a row.
///
/// The distinction from [`WorkloadSpec`] is who has acted: a spec *describes*
/// data, this *is* data. A frontend asks the registry what item type a row
/// wants, generates once at that type, and hands this over — so generation
/// happens before the row is entered rather than inside it.
#[derive(Debug, Clone)]
pub enum WorkloadData {
    /// Columns from `aqpbm-datagen`. The description rides along because the
    /// record names what the data was generated from, which columns alone do
    /// not carry.
    Generated {
        description: TableDescription,
        table: GeneratedTable,
    },
    /// A file to read. Still deferred to the row, because how the bytes are
    /// decoded depends on the item type — a `.bin` is a raw `i64` stream and
    /// only some rows can take it.
    File { path: String },
}

impl WorkloadSpec {
    /// Produce the data this spec describes, at `value_type` — the item type
    /// the row named. Called by the frontend, once, before the row is entered.
    ///
    /// This is why resolution comes first: `value_type` is an *answer* from
    /// the registry, so a caller cannot generate until it has asked.
    pub fn generate_at(&self, value_type: &str) -> Result<WorkloadData> {
        match self.describe(value_type) {
            Some(description) => {
                let table = description.generate()?;
                Ok(WorkloadData::Generated { description, table })
            }
            None => Ok(WorkloadData::File {
                path: self
                    .file_path()
                    .expect("describe() returns None only for the file variant")
                    .to_string(),
            }),
        }
    }

    /// Materialise at the item type `T`, which the row's `Accumulator::Item`
    /// already names. Generates and converts in one step; the split form is
    /// [`Self::generate_at`] followed by `T::materialise`.
    pub fn build<T: BenchItem>(&self) -> Result<T::Wk> {
        T::materialise(self.generate_at(T::DATA_TYPE)?)
    }

    /// The description to generate from at item type `item_type`, or `None` for
    /// a file-backed workload. See the variants above for why the two generated
    /// cases answer differently.
    pub fn describe(&self, item_type: &str) -> Option<TableDescription> {
        match self {
            WorkloadSpec::Generated(d) => Some(d.clone()),
            WorkloadSpec::Inline(d) => {
                let mut d = d.clone();
                for column in &mut d.column_spec {
                    column.data_type = item_type.to_string();
                }
                Some(d)
            }
            WorkloadSpec::File { .. } => None,
        }
    }

    /// The path a file-backed workload reads, if this is one.
    pub fn file_path(&self) -> Option<&str> {
        match self {
            WorkloadSpec::File { path } => Some(path),
            _ => None,
        }
    }
}

/// Why a `(impl, config)` cell cannot run: a workload it cannot obtain, or a
/// construction it cannot satisfy.
#[derive(Debug)]
pub enum RunError {
    Workload(anyhow::Error),
    Build(BuildError),
    /// A square of the grid nothing measures. Selection does not judge whether
    /// a combination is meaningful, so asking for one is legal; this is where
    /// the caller finds out there is nothing behind it.
    NotMeasured {
        operation: &'static str,
        metric: &'static str,
    },
    /// Merge was asked for with nothing to fold. A request that cannot be
    /// measured says so, the way an empty square does, instead of vanishing.
    NothingToFold { shards: usize },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Workload(e) => e.fmt(f),
            RunError::Build(e) => e.fmt(f),
            RunError::NotMeasured { operation, metric } => {
                write!(f, "nothing measures the {metric} of {operation}")
            }
            RunError::NothingToFold { shards } => write!(
                f,
                "merge folds {shards} shards into one, so there is nothing to fold; ask for at least 2"
            ),
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

/// What a record is labelled with: which algorithm, from which library.
///
/// Two strings, passed in. Core does not know the set of algorithms and has no
/// business deriving these — the registry that owns the row states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowLabel {
    pub algorithm: &'static str,
    pub impl_name: &'static str,
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the workload that carries
/// it, and how to build one from a [`WorkloadSpec`]. A row's
/// `Accumulator::Item` fixes the encoding before anything is generated.
pub trait BenchItem: Sized + Clone {
    type Wk: Workload<Item = Self>;

    /// Whether this item is materialised from a multi-column description
    /// instead of a single-column one. A `const`, so a registry can read which
    /// kind of workload a row wants off the row's type, without building one.
    const TAKES_COLUMNS: bool = false;

    /// The `data_type` a description has to state for this item — for a record,
    /// the type of its *value* column. Also a `const`, for the same reason.
    const DATA_TYPE: &'static str;

    /// Turn produced data into this row's workload. Takes the data by value:
    /// a million-row column is moved into the workload, never copied.
    fn materialise(data: WorkloadData) -> Result<Self::Wk>;
}

impl BenchItem for i64 {
    type Wk = I64Workload;
    const DATA_TYPE: &'static str = "i64";
    fn materialise(data: WorkloadData) -> Result<Self::Wk> {
        match data {
            WorkloadData::Generated { description, table } => {
                I64Workload::from_table(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadData::File { path } => I64Workload::load(std::path::Path::new(&path))
                .map_err(|e| anyhow::anyhow!("{}", e)),
        }
    }
}

impl BenchItem for f64 {
    type Wk = F64Workload;
    const DATA_TYPE: &'static str = "f64";
    fn materialise(data: WorkloadData) -> Result<Self::Wk> {
        match data {
            WorkloadData::Generated { description, table } => {
                F64Workload::from_table(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
            }
            // `.bin` is a raw i64 stream with no header; reading it as f64
            // would reinterpret the bytes, not convert them.
            WorkloadData::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a raw i64 stream; generate the workload \
                 instead to benchmark f64"
            )),
        }
    }
}

impl BenchItem for String {
    type Wk = StringWorkload;
    const DATA_TYPE: &'static str = "string";
    fn materialise(data: WorkloadData) -> Result<Self::Wk> {
        match data {
            WorkloadData::Generated { description, table } => {
                StringWorkload::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            // Decimal-formatted, the same rendering the i64-sourced path used.
            WorkloadData::File { path } => I64Workload::load(std::path::Path::new(&path))
                .map(|wk| StringWorkload::from_i64(&wk))
                .map_err(|e| anyhow::anyhow!("{}", e)),
        }
    }
}

impl BenchItem for Vec<u8> {
    type Wk = BytesWorkload;
    /// Materialised through the string path, so a description states `string`
    /// and the bytes are taken from it.
    const DATA_TYPE: &'static str = "string";
    fn materialise(data: WorkloadData) -> Result<Self::Wk> {
        Ok(BytesWorkload::from_strings(
            &<String as BenchItem>::materialise(data)?,
        ))
    }
}

/// The record item: only a multi-column description materialises one. A
/// single-column one is refused instead of being padded into a one-label
/// record, because the column count is what a grouped sketch's cost is a
/// function of.
impl<V: ColumnItem> BenchItem for Labeled<V> {
    type Wk = LabeledWorkload<V>;
    const TAKES_COLUMNS: bool = true;
    const DATA_TYPE: &'static str = V::NAME;
    fn materialise(data: WorkloadData) -> Result<Self::Wk> {
        match data {
            WorkloadData::Generated { description, table } => {
                LabeledWorkload::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadData::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a single-column stream; this row ingests \
                 labelled records, so it needs a `--spec` description with a \
                 label column before the value column"
            )),
        }
    }
}

// ---------- the hot-loop body + construction ----------

// The hot-loop body used to live here as `insert_body`, calling
// `Accumulator::update`. It is now the row's own `SketchOps::insert`, written in
// the wrapper file beside the sketch it drives.

/// Construct a fresh sketch. Construction is proven by the probe at the top of
/// each cell function, so the per-run factory unwraps.
fn built<S: InitSketch>(params: &ParamSet) -> S {
    S::init(params).expect("construction proven by the probe in the cell function")
}

// ---------- parallel-insert construction ----------

/// A parallel-insert wrapper: its constructor also takes the worker count (a
/// run knob, not a sketch parameter), so it cannot be an [`InitSketch`].
pub trait ParallelInit: Sized {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError>;
}

// ---------- running one cell ----------

/// Run the **timed** half of a cell: throughput / latency / CPU / memory. No
/// ground truth — the ground-truth calculator never touches the hot path.
pub fn run_cell<S, I, G, Ins>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    label: RowLabel,
    gt: Option<&G>,
    insert: Ins,
    ops: &SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: InitSketch + MemoryFootprint,
    I: BenchItem,
    G: GroundTruth<I>,
    Ins: FnMut(&mut S, &I),
{
    let wk = <I as BenchItem>::materialise(data)?;
    S::init(params)?; // probe: the cell fails here if it cannot build
    BenchRunner::new(cfg.clone(), &wk, label.algorithm, label.impl_name)
        .run::<S, _, G, _>(|| built::<S>(params), insert, gt, ops)
}

/// Run the **timed** half of a parallel-insert cell (workers from `cfg.threads`).
pub fn run_cell_parallel<S, I, G, Ins>(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    label: RowLabel,
    gt: Option<&G>,
    insert: Ins,
    ops: &SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + MemoryFootprint,
    I: BenchItem,
    G: GroundTruth<I>,
    Ins: FnMut(&mut S, &I),
{
    let wk = <I as BenchItem>::materialise(data)?;
    let workers = cfg.threads;
    S::build(params, workers)?; // probe
    BenchRunner::new(cfg.clone(), &wk, label.algorithm, label.impl_name).run::<S, _, G, _>(
        move || S::build(params, workers).expect("construction proven by the probe above"),
        insert,
        gt,
        ops,
    )
}

