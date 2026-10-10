//! The ordered statistic inside one subpopulation, in rank-error units.

use std::collections::BTreeMap;
use std::collections::HashMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{group_errors, group_labels, owned, write_groups, GroupKey};
use crate::accuracy::quantile::rank_err;
use crate::accuracy::{curve, f64_values, GroundTruth, GroupError};

/// Number of quantiles probed per group — the same grid the ungrouped
/// comparator uses, named once in [`super::quantile`].
use crate::accuracy::quantile::GRID_POINTS as GROUP_GRID_POINTS;

/// Ground truth for a grouped quantile sketch: the ordered statistic inside one
/// subpopulation, in rank-error units. The most expensive comparator here —
/// `groups * 101` estimate calls, unsampled. Too slow means fewer groups.
pub struct SubpopRankErrorGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

/// The ordered statistic inside each group, plus the groups worth probing.
pub struct SubpopRankTruth {
    per_group: HashMap<GroupKey, Vec<f64>>,
    /// Every group, shuffled: probe order must not hand the baseline the
    /// locality the encounter order would.
    probed: Vec<GroupKey>,
    items: usize,
    schema_width: usize,
}

impl GroundTruth for SubpopRankErrorGT {
    type Truth = SubpopRankTruth;
    /// One (group, fraction) question.
    type Probe = (GroupKey, f64);
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopRankTruth, DataGenError> {
        let labels = group_labels(table, &self.group_columns)?;
        let width = self.group_columns.len();
        let values = f64_values(table.column(self.value_column)?)?;

        // Every value the group carried, sorted. Not deduplicated, because rank
        // is over occurrences.
        let mut borrowed: HashMap<Vec<&str>, Vec<f64>> = HashMap::new();
        for (group, value) in labels.chunks(width).zip(&values) {
            match borrowed.get_mut(group) {
                Some(held) => held.push(*value),
                None => {
                    borrowed.insert(group.to_vec(), vec![*value]);
                }
            }
        }
        let mut per_group: HashMap<GroupKey, Vec<f64>> = HashMap::with_capacity(borrowed.len());
        for (group, mut held) in borrowed {
            held.sort_by(f64::total_cmp);
            per_group.insert(owned(&group, &self.group_columns), held);
        }

        let mut by_size: Vec<(GroupKey, u64)> = per_group
            .iter()
            .map(|(g, v)| (g.clone(), v.len() as u64))
            .collect();
        by_size.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let probed = curve::shuffled(&by_size);
        Ok(SubpopRankTruth {
            per_group,
            probed,
            items: table.row_num as usize,
            schema_width: self.value_column,
        })
    }

    fn probes(&self, truth: &SubpopRankTruth) -> Vec<(GroupKey, f64)> {
        let mut out = Vec::with_capacity(truth.probed.len() * GROUP_GRID_POINTS);
        for group in &truth.probed {
            match truth.per_group.get(group) {
                Some(sorted) if !sorted.is_empty() => {}
                _ => continue,
            }
            for i in 0..GROUP_GRID_POINTS {
                out.push((group.clone(), i as f64 / 100.0));
            }
        }
        out
    }

    fn score(
        &self,
        truth: &SubpopRankTruth,
        probes: &[(GroupKey, f64)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let (groups, max_err) = rank_errors(truth, probes, answers);
        let scored_groups = groups.len();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        metrics.insert(
            "mean_rank_err".into(),
            if scored_groups > 0 {
                groups.iter().map(|g| g.error).sum::<f64>() / scored_groups as f64
            } else {
                0.0
            },
        );
        metrics.insert("max_rank_err".into(), max_err);
        metrics.insert("grid_points".into(), GROUP_GRID_POINTS as f64);
        metrics.insert("items".into(), truth.items as f64);
        metrics.insert("probes".into(), scored_groups as f64);
        metrics.insert("subpopulations".into(), truth.per_group.len() as f64);
        metrics.insert("group_columns".into(), self.group_columns.len() as f64);
        write_groups(
            &groups,
            truth.items as u64,
            truth.schema_width,
            &mut metrics,
        );
        metrics
    }

    /// A group's error is its mean rank error over the grid, the number
    /// `mean_rank_err` averages over groups.
    fn per_group(
        &self,
        truth: &SubpopRankTruth,
        probes: &[(GroupKey, f64)],
        answers: &[f64],
    ) -> Vec<GroupError> {
        rank_errors(truth, probes, answers).0
    }
}

/// Each group's mean rank error over its grid, plus the largest rank error any
/// single probe saw.
fn rank_errors(
    truth: &SubpopRankTruth,
    probes: &[(GroupKey, f64)],
    answers: &[f64],
) -> (Vec<GroupError>, f64) {
    let mut max_err = 0.0f64;
    let mut groups = Vec::new();

    // The probe set is one contiguous grid per group, in order.
    for (chunk_p, chunk_a) in probes
        .chunks(GROUP_GRID_POINTS)
        .zip(answers.chunks(GROUP_GRID_POINTS))
    {
        let Some((group, _)) = chunk_p.first() else {
            continue;
        };
        let Some(sorted) = truth.per_group.get(group) else {
            continue;
        };
        let mut group_sum = 0.0f64;
        for ((_, q), est) in chunk_p.iter().zip(chunk_a) {
            let err = rank_err(sorted, *est, *q);
            group_sum += err;
            if err > max_err {
                max_err = err;
            }
        }
        groups.push((
            group.clone(),
            sorted.len() as u64,
            group_sum / GROUP_GRID_POINTS as f64,
        ));
    }
    (group_errors(groups.into_iter()), max_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::records;

    #[test]
    fn rank_error_truth_is_sorted_within_the_group() {
        let gt = SubpopRankErrorGT {
            group_columns: vec![0],
            value_column: 2,
        };
        let truth = gt
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.items, 6);
        assert_eq!(
            truth.per_group[&vec![Some("a".to_string())]],
            vec![10.0, 10.0, 20.0]
        );
        assert_eq!(
            truth.per_group[&vec![Some("b".to_string())]],
            vec![30.0, 30.0, 30.0]
        );
    }
}
