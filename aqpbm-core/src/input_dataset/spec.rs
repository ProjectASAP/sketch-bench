//! Where a benchmark's items come from, and what item type they materialise at.
//! [`InputDataSetSpec`] describes data, [`BenchItem`] is the item type a row ingests,
//! and [`InputDataSetSpec::build`] is the one entry point between them.

use crate::input_dataset::{
    F64InputDataSet, I64InputDataSet, InputDataSet, Labeled, LabeledInputDataSet,
};
use anyhow::Result;
use aqpbm_datagen::{ColumnItem, GeneratedTable, TableDescription};

// ---------- where items come from, and what they materialise to ----------

#[derive(Debug, Clone)]
pub enum InputDataSetSpec {
    Generated(TableDescription),
    Inline(TableDescription),
    File { path: String },
}

#[derive(Debug, Clone)]
pub enum InputDataSetData {
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

impl InputDataSetSpec {
    /// Materialise at the item type `T`, which the row's `insert` closure
    /// already pinned. Called from `crate::target` inside the row — the only point
    /// at which `T` is known, so the frontend cannot generate ahead of it.
    pub fn build<T: BenchItem>(&self) -> Result<T::Wk> {
        T::materialise(self.generate_at(T::DATA_TYPE)?)
    }

    /// Produce the data this spec describes, at `value_type` — the item type
    /// the row named.
    fn generate_at(&self, value_type: &str) -> Result<InputDataSetData> {
        match self.describe(value_type) {
            Some(description) => {
                let table = description.generate()?;
                Ok(InputDataSetData::Generated { description, table })
            }
            None => Ok(InputDataSetData::File {
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
            InputDataSetSpec::Generated(d) => Some(d.clone()),
            InputDataSetSpec::Inline(d) => {
                let mut d = d.clone();
                for column in &mut d.column_spec {
                    column.data_type = item_type.to_string();
                }
                Some(d)
            }
            InputDataSetSpec::File { .. } => None,
        }
    }

    /// The path a file-backed dataset reads, if this is one.
    fn file_path(&self) -> Option<&str> {
        match self {
            InputDataSetSpec::File { path } => Some(path),
            _ => None,
        }
    }
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the dataset that carries
/// it, and how to build one from a [`InputDataSetSpec`]. The row's own `insert` fixes
/// it, so binding a row's closures is what selects the impl below.
pub trait BenchItem: Sized + Clone {
    type Wk: InputDataSet<Item = Self>;

    /// The `data_type` a description has to state for this item — for a record,
    /// the type of its *value* column. A `const`, so a caller can read what a
    /// row wants off the row's type, without building one.
    const DATA_TYPE: &'static str;

    /// Turn produced data into this row's dataset. Takes the data by value:
    /// a million-row column is moved into the dataset, never copied.
    fn materialise(data: InputDataSetData) -> Result<Self::Wk>;
}

impl BenchItem for i64 {
    type Wk = I64InputDataSet;
    const DATA_TYPE: &'static str = "i64";
    fn materialise(data: InputDataSetData) -> Result<Self::Wk> {
        match data {
            InputDataSetData::Generated { description, table } => {
                I64InputDataSet::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            InputDataSetData::File { path } => I64InputDataSet::load(std::path::Path::new(&path))
                .map_err(|e| anyhow::anyhow!("{}", e)),
        }
    }
}

impl BenchItem for f64 {
    type Wk = F64InputDataSet;
    const DATA_TYPE: &'static str = "f64";
    fn materialise(data: InputDataSetData) -> Result<Self::Wk> {
        match data {
            InputDataSetData::Generated { description, table } => {
                F64InputDataSet::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            // `.bin` is a raw i64 stream with no header; reading it as f64
            // would reinterpret the bytes, not convert them.
            InputDataSetData::File { path } => Err(anyhow::anyhow!(
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
    type Wk = LabeledInputDataSet<V>;
    const DATA_TYPE: &'static str = V::NAME;
    fn materialise(data: InputDataSetData) -> Result<Self::Wk> {
        match data {
            InputDataSetData::Generated { description, table } => {
                LabeledInputDataSet::from_table(&description, table)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            InputDataSetData::File { path } => Err(anyhow::anyhow!(
                "--input {path} is a single-column stream; this row ingests \
                 labelled records, so it needs a `--spec` description with a \
                 label column before the value column"
            )),
        }
    }
}
