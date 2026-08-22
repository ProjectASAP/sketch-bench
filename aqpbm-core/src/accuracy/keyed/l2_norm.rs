use std::collections::BTreeMap;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{probes_over, score_over, truth_over};
use crate::accuracy::{CountedValue, GroundTruth};

pub struct KeyedL2NormGT<K> {
    pub key_column: usize,
    pub value_column: usize,
    key: PhantomData<K>,
}

impl<K> KeyedL2NormGT<K> {
    pub fn over_columns(key_column: usize, value_column: usize) -> Self {
        Self {
            key_column,
            value_column,
            key: PhantomData,
        }
    }
}

pub fn l2_norm(totals: &[i64]) -> f64 {
    totals
        .iter()
        .map(|&total| {
            let total = total as f64;
            total * total
        })
        .sum::<f64>()
        .sqrt()
}

impl<K: CountedValue> GroundTruth for KeyedL2NormGT<K> {
    type Truth = f64;
    type Probe = ();
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<f64, DataGenError> {
        truth_over::<K>(table, self.key_column, self.value_column, l2_norm)
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
    fn the_truth_is_the_root_of_the_summed_squared_totals() {
        let truth = truth_of(KeyedL2NormGT::<u64>::over_columns(0, 1), &keyed());
        assert!((truth - 128.0f64.sqrt()).abs() < 1e-12, "got {truth}");
    }

    #[test]
    fn a_key_width_is_a_tally_key_and_nothing_more() {
        assert_eq!(
            truth_of(
                KeyedL2NormGT::<String>::over_columns(0, 1),
                &keyed_by_string()
            ),
            truth_of(KeyedL2NormGT::<u64>::over_columns(0, 1), &keyed()),
        );
    }

    #[test]
    fn squaring_makes_the_sign_irrelevant() {
        assert_eq!(l2_norm(&[3, -4]), l2_norm(&[-3, 4]));
    }
}
