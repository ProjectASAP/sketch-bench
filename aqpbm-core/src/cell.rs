//! Where a benchmark's items come from, and what they materialise to.
//!
//! [`DatasetSpec`] describes data, [`DatasetData`] is data, and [`BenchItem`]
//! is the item type a row ingests — it names the dataset that carries it and
//! how to build one. Plus [`RunError`], which is what a caller sees when either
//! step cannot be done.
//!
//! Running a square is [`crate::ops::squares_for`]; timing it is
//! [`crate::measure`].

use crate::dataset::{Dataset, F64Dataset, I64Dataset, Labeled, LabeledDataset};
use anyhow::Result;
use aqpbm_datagen::{ColumnItem, GeneratedTable, TableDescription};

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
pub enum DatasetSpec {
    Generated(TableDescription),
    Inline(TableDescription),
    File { path: String },
}

/// A dataset that has already been produced, on its way to a row.
///
/// The distinction from [`DatasetSpec`] is who has acted: a spec *describes*
/// data, this *is* data. A frontend asks the registry what item type a row
/// wants, generates once at that type, and hands this over — so generation
/// happens before the row is entered rather than inside it.
#[derive(Debug, Clone)]
pub enum DatasetData {
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

impl DatasetSpec {
    /// Produce the data this spec describes, at `value_type` — the item type
    /// the row named. Called by the frontend, once, before the row is entered.
    ///
    /// This is why resolution comes first: `value_type` is an *answer* from
    /// the registry, so a caller cannot generate until it has asked.
    pub fn generate_at(&self, value_type: &str) -> Result<DatasetData> {
        match self.describe(value_type) {
            Some(description) => {
                let table = description.generate()?;
                Ok(DatasetData::Generated { description, table })
            }
            None => Ok(DatasetData::File {
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
    /// a file-backed dataset. See the variants above for why the two generated
    /// cases answer differently.
    pub fn describe(&self, item_type: &str) -> Option<TableDescription> {
        match self {
            DatasetSpec::Generated(d) => Some(d.clone()),
            DatasetSpec::Inline(d) => {
                let mut d = d.clone();
                for column in &mut d.column_spec {
                    column.data_type = item_type.to_string();
                }
                Some(d)
            }
            DatasetSpec::File { .. } => None,
        }
    }

    /// The path a file-backed dataset reads, if this is one.
    pub fn file_path(&self) -> Option<&str> {
        match self {
            DatasetSpec::File { path } => Some(path),
            _ => None,
        }
    }
}

/// Why a measurement could not be produced.
///
/// Only two ways left, both about the data: core no longer builds sketches, so
/// a construction failure is the registry's to report before it hands a body
/// over.
#[derive(Debug)]
pub enum RunError {
    Dataset(anyhow::Error),
    /// The body reported it could not run — a build the config cannot satisfy,
    /// a fold with nothing to fold. The registry words it; core carries it.
    Body(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Dataset(e) => e.fmt(f),
            RunError::Body(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for RunError {}

impl From<anyhow::Error> for RunError {
    fn from(e: anyhow::Error) -> Self {
        RunError::Dataset(e)
    }
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the dataset that carries
/// it, and how to build one from a [`DatasetSpec`]. A row's
/// `Accumulator::Item` fixes the encoding before anything is generated.
pub trait BenchItem: Sized + Clone {
    type Wk: Dataset<Item = Self>;

    /// The `data_type` a description has to state for this item — for a record,
    /// the type of its *value* column. A `const`, so a caller can read what a
    /// row wants off the row's type, without building one.
    const DATA_TYPE: &'static str;

    /// Turn produced data into this row's dataset. Takes the data by value:
    /// a million-row column is moved into the dataset, never copied.
    fn materialise(data: DatasetData) -> Result<Self::Wk>;
}

impl BenchItem for i64 {
    type Wk = I64Dataset;
    const DATA_TYPE: &'static str = "i64";
    fn materialise(data: DatasetData) -> Result<Self::Wk> {
        match data {
            DatasetData::Generated { description, table } => {
                I64Dataset::from_table(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
            }
            DatasetData::File { path } => {
                I64Dataset::load(std::path::Path::new(&path)).map_err(|e| anyhow::anyhow!("{}", e))
            }
        }
    }
}

impl BenchItem for f64 {
    type Wk = F64Dataset;
    const DATA_TYPE: &'static str = "f64";
    fn materialise(data: DatasetData) -> Result<Self::Wk> {
        match data {
            DatasetData::Generated { description, table } => {
                F64Dataset::from_table(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
            }
            // `.bin` is a raw i64 stream with no header; reading it as f64
            // would reinterpret the bytes, not convert them.
            DatasetData::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a raw i64 stream; generate the dataset \
                 instead to benchmark f64"
            )),
        }
    }
}

/// The record item: only a multi-column description materialises one. A
/// single-column one is refused instead of being padded into a one-label
/// record, because the column count is what a grouped sketch's cost is a
/// function of.
impl<V: ColumnItem> BenchItem for Labeled<V> {
    type Wk = LabeledDataset<V>;
    const DATA_TYPE: &'static str = V::NAME;
    fn materialise(data: DatasetData) -> Result<Self::Wk> {
        match data {
            DatasetData::Generated { description, table } => {
                LabeledDataset::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            DatasetData::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a single-column stream; this row ingests \
                 labelled records, so it needs a `--spec` description with a \
                 label column before the value column"
            )),
        }
    }
}

// The hot-loop body used to live here as `insert_body`, calling
// `Accumulator::update`. It is now the `insert` a row hands to
// `crate::ops::squares_for` as a generic parameter, written in the wrapper file
// beside the sketch it drives — see that module's note on why it is not an `fn`
// pointer.
