use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::frequency_vector::{
    per_group_over, probes_over, score_over, truth_over, SubpopVectorTruth,
};
use super::GroupKey;
use crate::accuracy::{GroundTruth, GroupError};

pub struct SubpopL1NormGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

pub fn l1_norm(counts: &[u64]) -> f64 {
    counts.iter().sum::<u64>() as f64
}

impl GroundTruth for SubpopL1NormGT {
    type Truth = SubpopVectorTruth;
    type Probe = GroupKey;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopVectorTruth, DataGenError> {
        truth_over(table, &self.group_columns, self.value_column, l1_norm)
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

    fn over(group_columns: Vec<usize>) -> SubpopL1NormGT {
        SubpopL1NormGT {
            group_columns,
            value_column: 2,
        }
    }

    #[test]
    fn l1_of_a_group_is_how_many_records_it_carried() {
        let truth = over(vec![0])
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.exact[&vec![Some("a".to_string())]], 3.0);
        assert_eq!(truth.exact[&vec![Some("b".to_string())]], 3.0);
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

    /// The per-group summary on a fixed case. Over both columns the groups
    /// carry 2, 1, 1 and 2 records; answering 2, 2, 1.5 and 3 misses them by
    /// 0, 1, 0.5 and 0.5 relatively, so sorted `[0, 0.5, 0.5, 1]`: mean 0.5,
    /// p50 (index round(1.5) = 2) 0.5, p90 (index round(2.7) = 3) 1, max 1.
    #[test]
    fn per_group_errors_are_summarised_over_groups() {
        let gt = over(vec![0, 1]);
        let table = records();
        let key = |a: &str, b: &str| vec![Some(a.to_string()), Some(b.to_string())];
        let mut answers = std::collections::HashMap::from([
            (key("a", "x"), 2.0),
            (key("a", "y"), 2.0),
            (key("b", "x"), 1.5),
            (key("b", "y"), 3.0),
        ]);
        let ask = |held: &mut std::collections::HashMap<GroupKey, f64>, p: &GroupKey| held[p];
        let cmp = crate::accuracy::score_with(&gt, &ask, &mut answers, &table);
        assert_eq!(cmp["err_mean"], 0.5);
        assert_eq!(cmp["err_p50"], 0.5);
        assert_eq!(cmp["err_p90"], 1.0);
        assert_eq!(cmp["err_max"], 1.0);
        assert_eq!(cmp["groups_scored"], 4.0);
        assert_eq!(cmp["schema_width"], 2.0);
        assert_eq!(cmp["records"], 6.0);
        assert_eq!(cmp["fanned_mass"], 18.0);

        let truth = gt.truth(&table).expect("the table has the named columns");
        let probes = gt.probes(&truth);
        let asked: Vec<f64> = probes.iter().map(|p| answers[p]).collect();
        let rows: Vec<(String, u64, Option<f64>)> = gt
            .per_group(&truth, &probes, &asked)
            .into_iter()
            .map(|g| (g.group, g.n_q, g.error))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("label0:a;label1:x".to_string(), 2, Some(0.0)),
                ("label0:b;label1:y".to_string(), 2, Some(0.5)),
                ("label0:a;label1:y".to_string(), 1, Some(1.0)),
                ("label0:b;label1:x".to_string(), 1, Some(0.5)),
            ]
        );
    }

    /// Column 1 alone is asked with column 0 left open, and named by column 1.
    #[test]
    fn a_group_on_a_later_column_leaves_the_earlier_ones_open() {
        let gt = over(vec![1]);
        let truth = gt
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.exact[&vec![None, Some("x".to_string())]], 3.0);
        let probes = gt.probes(&truth);
        let groups = gt.per_group(&truth, &probes, &vec![0.0; probes.len()]);
        let keys: Vec<&str> = groups.iter().map(|g| g.group.as_str()).collect();
        assert_eq!(keys, ["label1:x", "label1:y"]);
    }

    #[test]
    fn a_different_grouping_is_a_different_population() {
        let cmp = null_over(&over(vec![0, 1]), &records());
        assert_eq!(cmp["subpopulations"], 4.0);
        assert_eq!(cmp["group_columns"], 2.0);
    }
}
