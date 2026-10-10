//! The CDF inside one subpopulation, in absolute CDF error: the inverse of the
//! rank-error question, asked at a value and answered with a fraction.

use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{group_errors, write_groups, GroupKey, SubpopRankErrorGT, SubpopRankTruth};
use crate::accuracy::{GroundTruth, GroupError};

/// Thresholds probed per group: the group's own values at `phi` = 0.01..=0.99,
/// so every threshold sits inside the group's range whatever its scale.
const CDF_GRID_POINTS: usize = 99;

/// Ground truth for a grouped CDF sketch: `F_q(x)`, the share of group `q`'s
/// values at or below `x`, scored as `|F̂_q(x) − F_q(x)|`.
pub struct SubpopCdfErrorGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

/// `F(x)` over a sorted group: values at or below `x`, as a share.
fn cdf_at(sorted: &[f64], x: f64) -> f64 {
    sorted.partition_point(|v| *v <= x) as f64 / sorted.len() as f64
}

impl GroundTruth for SubpopCdfErrorGT {
    /// The same sorted values per group the rank-error comparator holds.
    type Truth = SubpopRankTruth;
    /// One (group, threshold) question.
    type Probe = (GroupKey, f64);
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopRankTruth, DataGenError> {
        SubpopRankErrorGT {
            group_columns: self.group_columns.clone(),
            value_column: self.value_column,
        }
        .truth(table)
    }

    /// The value at index `floor(phi * n)`, the index the exact quantile
    /// baseline answers `phi` with.
    fn probes(&self, truth: &SubpopRankTruth) -> Vec<(GroupKey, f64)> {
        let mut out = Vec::with_capacity(truth.probed.len() * CDF_GRID_POINTS);
        for group in &truth.probed {
            let Some(sorted) = truth.per_group.get(group).filter(|s| !s.is_empty()) else {
                continue;
            };
            for i in 1..=CDF_GRID_POINTS {
                let phi = i as f64 / 100.0;
                let idx = ((phi * sorted.len() as f64).floor() as usize).min(sorted.len() - 1);
                out.push((group.clone(), sorted[idx]));
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
        let (groups, max_err) = cdf_errors(truth, probes, answers);
        let scored_groups = groups.len();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        metrics.insert(
            "mean_cdf_err".into(),
            if scored_groups > 0 {
                groups.iter().filter_map(|g| g.error).sum::<f64>() / scored_groups as f64
            } else {
                0.0
            },
        );
        metrics.insert("max_cdf_err".into(), max_err);
        metrics.insert("grid_points".into(), CDF_GRID_POINTS as f64);
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

    /// A group's error is its mean absolute CDF error over the grid, the
    /// number `mean_cdf_err` averages over groups. Every group holds a record,
    /// so every group is scored.
    fn per_group(
        &self,
        truth: &SubpopRankTruth,
        probes: &[(GroupKey, f64)],
        answers: &[f64],
    ) -> Vec<GroupError> {
        cdf_errors(truth, probes, answers).0
    }
}

/// Each group's mean absolute CDF error over its grid, plus the largest error
/// any single probe saw.
fn cdf_errors(
    truth: &SubpopRankTruth,
    probes: &[(GroupKey, f64)],
    answers: &[f64],
) -> (Vec<GroupError>, f64) {
    let mut max_err = 0.0f64;
    let mut groups = Vec::new();
    // The probe set is one contiguous grid per group, in order.
    for (chunk_p, chunk_a) in probes
        .chunks(CDF_GRID_POINTS)
        .zip(answers.chunks(CDF_GRID_POINTS))
    {
        let Some((group, _)) = chunk_p.first() else {
            continue;
        };
        let Some(sorted) = truth.per_group.get(group) else {
            continue;
        };
        let mut group_sum = 0.0f64;
        for ((_, x), est) in chunk_p.iter().zip(chunk_a) {
            let err = (est - cdf_at(sorted, *x)).abs();
            group_sum += err;
            max_err = max_err.max(err);
        }
        groups.push((
            group.clone(),
            sorted.len() as u64,
            Some(group_sum / CDF_GRID_POINTS as f64),
        ));
    }
    (group_errors(groups.into_iter()), max_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::records;

    fn over(group_columns: Vec<usize>) -> SubpopCdfErrorGT {
        SubpopCdfErrorGT {
            group_columns,
            value_column: 2,
        }
    }

    /// Answering each threshold with its exact share scores zero, and the
    /// thresholds are the group's own values: `a` holds 10, 10, 20, so
    /// `F_a(10)` is 2/3 and `F_a(20)` is 1, at or below, ties included.
    #[test]
    fn the_exact_cdf_scores_zero() {
        let gt = over(vec![0]);
        let table = records();
        let mut truth = gt.truth(&table).expect("the table has the named columns");
        let probes = gt.probes(&truth);
        assert_eq!(probes.len(), 2 * CDF_GRID_POINTS);
        let a = vec![Some("a".to_string())];
        let sorted = truth.per_group[&a].clone();
        assert_eq!(cdf_at(&sorted, 10.0), 2.0 / 3.0);
        assert_eq!(cdf_at(&sorted, 20.0), 1.0);
        let cmp = crate::accuracy::score_with(
            &gt,
            &|t: &mut SubpopRankTruth, p: &(GroupKey, f64)| cdf_at(&t.per_group[&p.0], p.1),
            &mut truth,
            &table,
        );
        assert_eq!(cmp["mean_cdf_err"], 0.0);
        assert_eq!(cmp["err_max"], 0.0);
        assert_eq!(cmp["groups_scored"], 2.0);
    }

    /// Answering 0 everywhere misses each threshold by its whole share: over
    /// `b`, which holds only 30, every threshold is 30 and `F_b(30)` is 1.
    #[test]
    fn a_zero_answer_misses_by_the_share() {
        let gt = over(vec![0]);
        let table = records();
        let truth = gt.truth(&table).expect("the table has the named columns");
        let probes = gt.probes(&truth);
        let groups = gt.per_group(&truth, &probes, &vec![0.0; probes.len()]);
        let b = groups
            .iter()
            .find(|g| g.group == "label0:b")
            .expect("b is scored");
        assert_eq!(b.error, Some(1.0));
        assert_eq!(b.n_q, 3);
    }
}
