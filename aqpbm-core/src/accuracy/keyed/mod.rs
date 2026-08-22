mod cardinality;
mod entropy;
mod l1_norm;
mod l2_norm;

pub use cardinality::{cardinality, KeyedCardinalityGT};
pub use entropy::{entropy, KeyedEntropyGT};
pub use l1_norm::{l1_norm, KeyedL1NormGT};
pub use l2_norm::{l2_norm, KeyedL2NormGT};

use std::collections::BTreeMap;

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable};

use crate::accuracy::{scalar_error, CountedValue};

const KEYED_QUERY_REPEATS: usize = 4096;

fn keyed_totals<K: CountedValue>(
    table: &GeneratedTable,
    key_column: usize,
    value_column: usize,
) -> Result<BTreeMap<K::CountKey, i64>, DataGenError> {
    let keys = K::column_slice(table.column(key_column)?)?;
    let values = i64::column_slice(table.column(value_column)?)?;
    let mut totals: BTreeMap<K::CountKey, i64> = BTreeMap::new();
    for (key, value) in keys.iter().zip(values) {
        *totals.entry(key.count_key()).or_insert(0) += value;
    }
    Ok(totals)
}

fn truth_over<K: CountedValue>(
    table: &GeneratedTable,
    key_column: usize,
    value_column: usize,
    of_totals: fn(&[i64]) -> f64,
) -> Result<f64, DataGenError> {
    let totals = keyed_totals::<K>(table, key_column, value_column)?;
    let totals: Vec<i64> = totals.into_values().collect();
    Ok(of_totals(&totals))
}

fn probes_over() -> Vec<()> {
    vec![(); KEYED_QUERY_REPEATS]
}

fn score_over(truth: f64, answers: &[f64]) -> BTreeMap<String, f64> {
    scalar_error(truth, answers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::{table_of, GroundTruth};
    use aqpbm_datagen::ColumnData;

    pub(super) fn keyed() -> GeneratedTable {
        table_of(
            &["key", "value"],
            vec![
                ColumnData::Unsigned64(vec![1, 1, 2, 3]),
                ColumnData::Int64(vec![3, 5, 8, 0]),
            ],
        )
    }

    pub(super) fn keyed_by_string() -> GeneratedTable {
        table_of(
            &["key", "value"],
            vec![
                ColumnData::String(["a", "a", "b", "c"].iter().map(|k| k.to_string()).collect()),
                ColumnData::Int64(vec![3, 5, 8, 0]),
            ],
        )
    }

    pub(super) fn truth_of<G: GroundTruth<Truth = f64>>(gt: G, table: &GeneratedTable) -> f64 {
        gt.truth(table)
            .expect("both columns are the types a keyed row reads")
    }

    #[test]
    fn one_question_is_asked_the_same_number_of_times_by_all_four() {
        let repeats = KEYED_QUERY_REPEATS;
        assert_eq!(
            KeyedL1NormGT::<u64>::over_columns(0, 1).probes(&0.0).len(),
            repeats
        );
        assert_eq!(
            KeyedEntropyGT::<u64>::over_columns(0, 1).probes(&0.0).len(),
            repeats
        );
    }

    #[test]
    fn a_key_column_the_row_cannot_read_is_named_not_ignored() {
        let Err(err) = KeyedL1NormGT::<u64>::over_columns(0, 1).truth(&keyed_by_string()) else {
            panic!("a string key column read at u64 must be refused");
        };
        let err = err.to_string();
        assert!(err.contains("u64"), "error should name the width: {err}");
    }
}
