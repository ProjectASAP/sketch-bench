use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::frequency_vector::{
    per_group_over, probes_over, score_over, truth_over, SubpopVectorTruth,
};
use super::GroupKey;
use crate::accuracy::{GroundTruth, GroupError};

pub struct SubpopEntropyGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

pub fn entropy(counts: &[u64]) -> f64 {
    let l1: f64 = counts.iter().sum::<u64>() as f64;
    if l1 <= 0.0 {
        return 0.0;
    }
    -counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let share = *c as f64 / l1;
            share * share.log2()
        })
        .sum::<f64>()
}

impl GroundTruth for SubpopEntropyGT {
    type Truth = SubpopVectorTruth;
    type Probe = GroupKey;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopVectorTruth, DataGenError> {
        truth_over(table, &self.group_columns, self.value_column, entropy)
    }

    fn probes(&self, truth: &SubpopVectorTruth) -> Vec<GroupKey> {
        probes_over(truth)
    }

    fn score(
        &self,
        truth: &SubpopVectorTruth,
        probes: &[GroupKey],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        score_over(truth, probes, answers, self.group_columns.len())
    }

    fn per_group(
        &self,
        truth: &SubpopVectorTruth,
        probes: &[GroupKey],
        answers: &[f64],
    ) -> Vec<GroupError> {
        per_group_over(truth, probes, answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::{exact_over, null_over, records};

    fn over(group_columns: Vec<usize>) -> SubpopEntropyGT {
        SubpopEntropyGT {
            group_columns,
            value_column: 2,
        }
    }

    #[test]
    fn entropy_is_zero_when_the_group_held_one_value() {
        let truth = over(vec![0])
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.exact[&vec![Some("b".to_string())]], 0.0);
        let expected = -(2.0 / 3.0 * (2.0f64 / 3.0).log2() + 1.0 / 3.0 * (1.0f64 / 3.0).log2());
        assert!((truth.exact[&vec![Some("a".to_string())]] - expected).abs() < 1e-12);
    }

    #[test]
    fn null_estimator_misses_by_the_whole_entropy() {
        let gt = over(vec![0]);
        let truth = gt
            .truth(&records())
            .expect("the table has the named columns");
        let mean: f64 = truth.exact.values().sum::<f64>() / truth.exact.len() as f64;
        let cmp = null_over(&gt, &records());
        assert!((cmp["aae_all"] - mean).abs() < 1e-12, "{}", cmp["aae_all"]);
        assert!(cmp["are_all"] <= 1.0, "{}", cmp["are_all"]);
    }

    #[test]
    fn exact_estimator_scores_zero() {
        let cmp = exact_over(&over(vec![0]), &records());
        assert_eq!(cmp["are_all"], 0.0);
        assert_eq!(cmp["aae_all"], 0.0);
    }

    #[test]
    fn a_different_grouping_is_a_different_population() {
        let cmp = null_over(&over(vec![0, 1]), &records());
        assert_eq!(cmp["subpopulations"], 4.0);
        assert_eq!(cmp["group_columns"], 2.0);
    }
}
