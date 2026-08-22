use std::collections::BTreeMap;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{probes_over, score_over, truth_over};
use crate::accuracy::{CountedValue, GroundTruth};

pub struct KeyedEntropyGT<K> {
    pub key_column: usize,
    pub value_column: usize,
    key: PhantomData<K>,
}

impl<K> KeyedEntropyGT<K> {
    pub fn over_columns(key_column: usize, value_column: usize) -> Self {
        Self {
            key_column,
            value_column,
            key: PhantomData,
        }
    }
}

pub fn entropy(totals: &[i64]) -> f64 {
    let weights: Vec<f64> = totals
        .iter()
        .filter(|&&total| total != 0)
        .map(|total| total.unsigned_abs() as f64)
        .collect();
    let l1: f64 = weights.iter().sum();
    if l1 <= 0.0 {
        return 0.0;
    }
    -weights
        .iter()
        .map(|weight| {
            let share = weight / l1;
            share * share.log2()
        })
        .sum::<f64>()
}

impl<K: CountedValue> GroundTruth for KeyedEntropyGT<K> {
    type Truth = f64;
    type Probe = ();
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<f64, DataGenError> {
        truth_over::<K>(table, self.key_column, self.value_column, entropy)
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
    fn the_truth_is_the_shannon_entropy_of_the_key_shares() {
        let truth = truth_of(KeyedEntropyGT::<u64>::over_columns(0, 1), &keyed());
        assert!((truth - 1.0).abs() < 1e-12, "got {truth}");
    }

    #[test]
    fn a_key_width_is_a_tally_key_and_nothing_more() {
        assert_eq!(
            truth_of(
                KeyedEntropyGT::<String>::over_columns(0, 1),
                &keyed_by_string()
            ),
            truth_of(KeyedEntropyGT::<u64>::over_columns(0, 1), &keyed()),
        );
    }

    #[test]
    fn a_stream_carrying_no_weight_has_no_shares() {
        assert_eq!(entropy(&[0, 0]), 0.0);
        assert_eq!(entropy(&[]), 0.0);
    }
}
