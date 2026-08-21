//! Ground truth for the grouped sketches: a statistic taken *within* one
//! subpopulation, one comparator per statistic.

mod cardinality;
mod entropy;
mod frequency;
mod frequency_vector;
mod l1_norm;
mod l2_norm;
mod rank_error;

pub use cardinality::{SubpopCardTruth, SubpopCardinalityGT};
pub use entropy::{entropy, SubpopEntropyGT};
pub use frequency::{SubpopFreqTruth, SubpopFrequencyGT};
pub use frequency_vector::SubpopVectorTruth;
pub use l1_norm::{l1_norm, SubpopL1NormGT};
pub use l2_norm::{l2_norm, SubpopL2NormGT};
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

#[cfg(test)]
fn null_over<G>(gt: &G, table: &GeneratedTable) -> std::collections::BTreeMap<String, f64>
where
    G: crate::accuracy::GroundTruth<Probe = Group, Answer = f64>,
{
    crate::accuracy::score_with(gt, &|_: &mut (), _: &Group| 0.0, &mut (), table)
}

#[cfg(test)]
fn exact_over<G>(gt: &G, table: &GeneratedTable) -> std::collections::BTreeMap<String, f64>
where
    G: crate::accuracy::GroundTruth<Truth = SubpopVectorTruth, Probe = Group, Answer = f64>,
{
    let mut exact = gt
        .truth(table)
        .expect("the test's table matches its comparator")
        .exact;
    crate::accuracy::score_with(
        gt,
        &|held: &mut std::collections::HashMap<Group, f64>, p: &Group| {
            held.get(p).copied().unwrap_or(0.0)
        },
        &mut exact,
        table,
    )
}
