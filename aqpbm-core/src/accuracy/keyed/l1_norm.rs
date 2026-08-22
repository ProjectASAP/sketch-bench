use std::collections::BTreeMap;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{probes_over, score_over, truth_over};
use crate::accuracy::{CountedValue, GroundTruth};

pub struct KeyedL1NormGT<K> {
    pub key_column: usize,
    pub value_column: usize,
    key: PhantomData<K>,
}

impl<K> KeyedL1NormGT<K> {
    pub fn over_columns(key_column: usize, value_column: usize) -> Self {
        Self {
            key_column,
            value_column,
            key: PhantomData,
        }
    }
}

pub fn l1_norm(totals: &[i64]) -> f64 {
    totals.iter().map(|total| total.unsigned_abs() as f64).sum()
}

impl<K: CountedValue> GroundTruth for KeyedL1NormGT<K> {
    type Truth = f64;
    type Probe = ();
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<f64, DataGenError> {
        truth_over::<K>(table, self.key_column, self.value_column, l1_norm)
    }

    fn probes(&self, _truth: &f64) -> Vec<()> {
        probes_over()
    }

    fn score(&self, truth: &f64, _probes: &[()], answers: &[f64]) -> BTreeMap<String, f64> {
        score_over(*truth, answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::keyed::tests::{keyed, keyed_by_string, truth_of};

    #[test]
    fn the_truth_is_the_summed_per_key_totals() {
        let truth = truth_of(KeyedL1NormGT::<u64>::over_columns(0, 1), &keyed());
        assert!((truth - 16.0).abs() < 1e-12, "got {truth}");
    }

    #[test]
    fn a_key_width_is_a_tally_key_and_nothing_more() {
        assert_eq!(
            truth_of(
                KeyedL1NormGT::<String>::over_columns(0, 1),
                &keyed_by_string()
            ),
            truth_of(KeyedL1NormGT::<u64>::over_columns(0, 1), &keyed()),
        );
    }

    #[test]
    fn a_negative_total_carries_its_magnitude() {
        assert_eq!(l1_norm(&[3, -5]), 8.0);
    }
}
