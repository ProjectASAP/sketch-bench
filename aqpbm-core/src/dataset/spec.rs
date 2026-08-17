//! Where a benchmark's items come from, and what item type they materialise at.
//!
//! [`DatasetSpec`] describes data and [`BenchItem`] is the item type a row
//! ingests — it names the dataset that carries it and how to build one.
//! [`DatasetSpec::build`] is the one entry point: a row hands over the spec it
//! was given and gets back a dataset at its own item type.
//!
//! Running a square is [`crate::ops::squares_for`]; timing it is
//! [`crate::measure`](mod@crate::measure).

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

/// Produced data, on its way to a [`BenchItem`].
///
/// The intermediate inside [`DatasetSpec::build`], and nothing more: it carries
/// the result of generating across to the `materialise` that decodes it. The
/// distinction from [`DatasetSpec`] is who has acted — a spec *describes* data,
/// this *is* data.
///
/// It stays `pub` only because [`BenchItem::materialise`] takes one and that
/// trait is a public bound on every `ops::squares_*`. Nothing outside this
/// module constructs or matches on it; generation is not a step a caller
/// performs.
#[derive(Debug, Clone)]
pub enum DatasetData {
    /// Columns from `aqpbm-datagen`. The description rides along because the
    /// record names what the data was generated from, which columns alone do
    /// not carry.
    Generated {
        description: TableDescription,
        table: GeneratedTable,
    },
    /// A file to read. Still deferred to the item type, because how the bytes
    /// are decoded depends on it — a `.bin` is a raw `i64` stream and only some
    /// rows can take it.
    File { path: String },
}

impl DatasetSpec {
    /// Materialise at the item type `T`, which the row's `insert` closure
    /// already pinned.
    ///
    /// This is the whole public surface. It is called from `crate::ops`, inside
    /// the row, *after* the frontend has bound a pair of argv strings to a
    /// concrete set of closures — which is the only point at which `T` is known.
    /// The frontend cannot generate ahead of this without the row publishing its
    /// item type back out.
    pub fn build<T: BenchItem>(&self) -> Result<T::Wk> {
        T::materialise(self.generate_at(T::DATA_TYPE)?)
    }

    /// Produce the data this spec describes, at `value_type` — the item type
    /// the row named.
    fn generate_at(&self, value_type: &str) -> Result<DatasetData> {
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

    /// The description to generate from at item type `item_type`, or `None` for
    /// a file-backed dataset. See the variants above for why the two generated
    /// cases answer differently.
    fn describe(&self, item_type: &str) -> Option<TableDescription> {
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
    fn file_path(&self) -> Option<&str> {
        match self {
            DatasetSpec::File { path } => Some(path),
            _ => None,
        }
    }
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the dataset that carries
/// it, and how to build one from a [`DatasetSpec`].
///
/// The item type is fixed by the row's own `insert` — `insert_cms_datasketches`
/// takes a `&i64`, `insert_hydra_cms` takes a `&Labeled<i64>` — so binding a
/// row's closures is what selects the impl below, and nothing is generated
/// before that has happened.
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
