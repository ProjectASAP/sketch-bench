//! Ground truth for the grouped sketches: a statistic taken *within* one
//! subpopulation, one comparator per statistic.

mod cardinality;
mod frequency;
mod rank_error;

pub use cardinality::{SubpopCardTruth, SubpopCardinalityGT};
pub use frequency::{SubpopFreqTruth, SubpopFrequencyGT};
pub use rank_error::{SubpopRankErrorGT, SubpopRankTruth};

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable};

pub type Group = Vec<String>;

fn group_labels<'a>(
    table: &'a GeneratedTable,
    columns: &[usize],
) -> Result<Vec<&'a str>, DataGenError> {
    if columns.is_empty() {
        return Err(DataGenError::BadParam(
            "a subpopulation is taken over at least one grouping column, and none were named"
                .into(),
        ));
    }
    let grouping: Vec<&[String]> = columns
        .iter()
        .map(|&i| String::column_slice(table.column(i)?))
        .collect::<Result<Vec<_>, DataGenError>>()?;
    let rows = table.row_num as usize;
    let mut flat = Vec::with_capacity(rows * columns.len());
    for r in 0..rows {
        for column in &grouping {
            flat.push(column[r].as_str());
        }
    }
    Ok(flat)
}

fn owned(group: &[&str]) -> Group {
    group.iter().map(|label| (*label).to_string()).collect()
}

/// key1 ∈ {a, b}, key2 ∈ {x, y}; the value column is what gets counted.
#[cfg(test)]
fn records() -> GeneratedTable {
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;

    let rows = [
        ("a", "x", 10),
        ("a", "y", 10),
        ("a", "x", 20),
        ("b", "x", 30),
        ("b", "y", 30),
        ("b", "y", 30),
    ];
    table_of(
        &["key1", "key2", "value"],
        vec![
            ColumnData::String(rows.iter().map(|r| r.0.to_string()).collect()),
            ColumnData::String(rows.iter().map(|r| r.1.to_string()).collect()),
            ColumnData::Int64(rows.iter().map(|r| r.2).collect()),
        ],
    )
}
