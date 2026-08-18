//! Where a benchmark's items come from, and what item type they materialise at.
//! [`InputDataSetSpec`] describes data, [`BenchItem`] is the item type a row ingests,
//! and [`InputDataSetSpec::build`] is the one entry point between them.

use crate::input_dataset::{labeled, numeric, Labeled, Materialised};
use anyhow::Result;
use aqpbm_datagen::{ColumnItem, GeneratedTable, TableDescription};

// ---------- where items come from, and what they materialise to ----------

#[derive(Debug, Clone)]
pub enum InputDataSetSpec {
    Generated(TableDescription),
    Inline(TableDescription),
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
}

impl InputDataSetSpec {
    /// Materialise at the item type `T`, which the row's `insert` closure
    /// already pinned. The two halves are also callable apart, which is what a
    /// frontend does: it asks the row what it ingests, produces that with
    /// [`generate_at`](Self::generate_at), and hands the data over for the row
    /// to [`materialise`](BenchItem::materialise).
    pub fn build<T: BenchItem>(&self) -> Result<Materialised<T>> {
        T::materialise(self.generate_at(T::DATA_TYPE)?)
    }

    /// Produce the data this spec describes, at `value_type` — the item type
    /// the row named.
    pub fn generate_at(&self, value_type: &str) -> Result<InputDataSetData> {
        let description = self.describe(value_type);
        let table = description.generate()?;
        Ok(InputDataSetData::Generated { description, table })
    }

    /// The description to generate from at item type `item_type`. See the
    /// variants above for why the two cases answer differently.
    fn describe(&self, item_type: &str) -> TableDescription {
        match self {
            InputDataSetSpec::Generated(d) => d.clone(),
            InputDataSetSpec::Inline(d) => {
                let mut d = d.clone();
                for column in &mut d.column_spec {
                    column.data_type = item_type.to_string();
                }
                d
            }
        }
    }
}

// ---------- the item axis ----------

/// An item type a benchmark can be run over: it names the dataset that carries
/// it, and how to build one from a [`InputDataSetSpec`]. The row's own `insert` fixes
/// it, so binding a row's closures is what selects the impl below.
pub trait BenchItem: Sized + Clone {
    /// The `data_type` a description has to state for this item — for a record,
    /// the type of its *value* column. A `const`, so a caller can read what a
    /// row wants off the row's type, without building one.
    const DATA_TYPE: &'static str;

    /// Turn produced data into this row's item stream. Takes the data by value:
    /// a million-row column is moved into the stream, never copied.
    fn materialise(data: InputDataSetData) -> Result<Materialised<Self>>;
}

impl BenchItem for i64 {
    const DATA_TYPE: &'static str = "i64";
    fn materialise(data: InputDataSetData) -> Result<Materialised<Self>> {
        let InputDataSetData::Generated { description, table } = data;
        numeric::from_table::<i64>(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
    }
}

impl BenchItem for f64 {
    const DATA_TYPE: &'static str = "f64";
    fn materialise(data: InputDataSetData) -> Result<Materialised<Self>> {
        let InputDataSetData::Generated { description, table } = data;
        numeric::from_table::<f64>(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
    }
}

/// The record item: only a multi-column description materialises one. A
/// single-column one is refused instead of being padded into a one-label
/// record, because the column count is what a grouped sketch's cost is a
/// function of.
impl<V: ColumnItem> BenchItem for Labeled<V> {
    const DATA_TYPE: &'static str = V::NAME;
    fn materialise(data: InputDataSetData) -> Result<Materialised<Self>> {
        let InputDataSetData::Generated { description, table } = data;
        labeled::from_table::<V>(&description, table).map_err(|e| anyhow::anyhow!("{}", e))
    }
}
