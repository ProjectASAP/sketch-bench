//! The sum of the values inside one subpopulation: the weighted L1 norm, each
//! value its own weight.

use std::collections::BTreeMap;
use std::collections::HashMap;

use aqpbm_datagen::{ColumnData, DataGenError, GeneratedTable};

use super::frequency_vector::{per_group_over, probes_over, score_over, truth_of};
use super::{group_labels, owned, GroupKey, SubpopVectorTruth};
use crate::accuracy::{GroundTruth, GroupError};

pub struct SubpopSumGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

/// Each group's sum, over its record count. The value is an integer weight,
/// so an `f64` or text value column is refused by its type.
fn sums_per_group(
    table: &GeneratedTable,
    group_columns: &[usize],
    value_column: usize,
) -> Result<SubpopVectorTruth, DataGenError> {
    let labels = group_labels(table, group_columns)?;
    let values: Vec<f64> = match table.column(value_column)? {
        ColumnData::Int64(v) => v.iter().map(|&x| x as f64).collect(),
        ColumnData::Unsigned64(v) => v.iter().map(|&x| x as f64).collect(),
        other => {
            return Err(DataGenError::TypeMismatch {
                held: other.kind(),
                wanted: "an integer value column (i64 or u64), the weights a sum adds",
            })
        }
    };
    let mut per_group: HashMap<Vec<&str>, (f64, u64)> = HashMap::new();
    for (group, value) in labels.chunks(group_columns.len()).zip(&values) {
        match per_group.get_mut(group) {
            Some((sum, n)) => {
                *sum += value;
                *n += 1;
            }
            None => {
                per_group.insert(group.to_vec(), (*value, 1));
            }
        }
    }
    let (mut sums, mut sizes) = (HashMap::new(), HashMap::new());
    for (group, (sum, n)) in per_group {
        let key = owned(&group, group_columns);
        sums.insert(key.clone(), sum);
        sizes.insert(key, n);
    }
    Ok(truth_of(sums, sizes, table.row_num, value_column))
}

impl GroundTruth for SubpopSumGT {
    type Truth = SubpopVectorTruth;
    type Probe = GroupKey;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopVectorTruth, DataGenError> {
        sums_per_group(table, &self.group_columns, self.value_column)
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

    fn over(group_columns: Vec<usize>) -> SubpopSumGT {
        SubpopSumGT {
            group_columns,
            value_column: 2,
        }
    }

    /// `a` carries 10, 10 and 20; `b` 30 three times. Its L1 sibling would
    /// answer 3 for both, the record count.
    #[test]
    fn the_sum_of_a_group_adds_its_values() {
        let truth = over(vec![0])
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.exact[&vec![Some("a".to_string())]], 40.0);
        assert_eq!(truth.exact[&vec![Some("b".to_string())]], 90.0);
        let probes = over(vec![0]).probes(&truth);
        let groups = over(vec![0]).per_group(&truth, &probes, &vec![0.0; probes.len()]);
        let sizes: Vec<u64> = groups.iter().map(|g| g.n_q).collect();
        assert_eq!(sizes, [3, 3], "n_q is the group's records, not its sum");
    }

    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let cmp = null_over(&over(vec![0]), &records());
        assert!((cmp["are_all"] - 1.0).abs() < 1e-12, "{}", cmp["are_all"]);
    }

    #[test]
    fn exact_estimator_scores_zero() {
        let cmp = exact_over(&over(vec![0, 1]), &records());
        assert_eq!(cmp["are_all"], 0.0);
        assert_eq!(cmp["err_max"], 0.0);
    }

    /// A group whose values sum to zero has no relative error: it is listed
    /// unscored, as a group with no distinct value is for cardinality.
    #[test]
    fn a_zero_sum_group_is_listed_unscored() {
        use crate::accuracy::table_of;
        let table = table_of(
            &["key1", "value"],
            vec![
                ColumnData::String(vec!["a".into(), "b".into(), "b".into()]),
                ColumnData::Int64(vec![4, 0, 0]),
            ],
        );
        let gt = SubpopSumGT {
            group_columns: vec![0],
            value_column: 1,
        };
        let truth = gt.truth(&table).expect("the table has the named columns");
        let probes = gt.probes(&truth);
        let rows: Vec<(String, u64, Option<f64>)> = gt
            .per_group(&truth, &probes, &vec![0.0; probes.len()])
            .into_iter()
            .map(|g| (g.group, g.n_q, g.error))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("label0:b".to_string(), 2, None),
                ("label0:a".to_string(), 1, Some(1.0)),
            ]
        );
    }

    #[test]
    fn a_float_value_column_is_refused_by_its_type() {
        use crate::accuracy::table_of;
        let table = table_of(
            &["key1", "value"],
            vec![
                ColumnData::String(vec!["a".into()]),
                ColumnData::Float64(vec![1.5]),
            ],
        );
        let gt = SubpopSumGT {
            group_columns: vec![0],
            value_column: 1,
        };
        let Err(err) = gt.truth(&table).map(|_| ()) else {
            panic!("an f64 value column must be refused, not summed");
        };
        let err = err.to_string();
        assert!(err.contains("f64") && err.contains("integer"), "{err}");
    }
}
