use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{keyed_totals, scalar_error, GroundTruth, KEYED_QUERY_REPEATS};

#[derive(Debug, Default, Clone, Copy)]
pub struct KeyedL1NormGT {
    pub key_column: usize,
    pub value_column: usize,
}

impl GroundTruth for KeyedL1NormGT {
    type Truth = f64;
    type Probe = ();
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<f64, DataGenError> {
        let totals = keyed_totals(table, self.key_column, self.value_column)?;
        Ok(totals
            .values()
            .map(|total| total.unsigned_abs() as f64)
            .sum())
    }

    fn probes(&self, _truth: &f64) -> Vec<()> {
        vec![(); KEYED_QUERY_REPEATS]
    }

    fn score(&self, truth: &f64, _probes: &[()], answers: &[f64]) -> BTreeMap<String, f64> {
        scalar_error(*truth, answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;

    fn keyed() -> GeneratedTable {
        table_of(
            &["key", "value"],
            vec![
                ColumnData::Unsigned64(vec![1, 1, 2, 3]),
                ColumnData::Int64(vec![3, 5, 8, 0]),
            ],
        )
    }

    #[test]
    fn the_truth_is_the_summed_per_key_totals() {
        let gt = KeyedL1NormGT {
            key_column: 0,
            value_column: 1,
        };
        let truth = gt
            .truth(&keyed())
            .expect("both columns are the types a keyed row reads");
        assert!((truth - (16.0)).abs() < 1e-12, "got {truth}");
    }
}
