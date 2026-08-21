use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::frequency_vector::{probes_over, score_over, truth_over, SubpopVectorTruth};
use super::Group;
use crate::accuracy::GroundTruth;

pub struct SubpopL2NormGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

pub fn l2_norm(counts: &[u64]) -> f64 {
    counts
        .iter()
        .map(|c| (*c as f64) * (*c as f64))
        .sum::<f64>()
        .sqrt()
}

impl GroundTruth for SubpopL2NormGT {
    type Truth = SubpopVectorTruth;
    type Probe = Group;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopVectorTruth, DataGenError> {
        truth_over(table, &self.group_columns, self.value_column, l2_norm)
    }

    fn probes(&self, truth: &SubpopVectorTruth) -> Vec<Group> {
        probes_over(truth)
    }

    fn score(
        &self,
        truth: &SubpopVectorTruth,
        probes: &[Group],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        score_over(truth, probes, answers, self.group_columns.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::{exact_over, null_over, records};

    fn over(group_columns: Vec<usize>) -> SubpopL2NormGT {
        SubpopL2NormGT {
            group_columns,
            value_column: 2,
        }
    }

    #[test]
    fn l2_is_the_root_of_the_summed_squares_of_the_counts() {
        let truth = over(vec![0])
            .truth(&records())
            .expect("the table has the named columns");
        assert!((truth.exact[&vec!["a".to_string()]] - 5.0f64.sqrt()).abs() < 1e-12);
        assert_eq!(truth.exact[&vec!["b".to_string()]], 3.0);
    }

    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let cmp = null_over(&over(vec![0]), &records());
        assert!((cmp["are_all"] - 1.0).abs() < 1e-12, "{}", cmp["are_all"]);
        assert!(cmp["aae_all"] > 0.0);
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
