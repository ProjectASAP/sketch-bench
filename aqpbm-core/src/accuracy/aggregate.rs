//! Exact-aggregate ground truth: the `*_over_time` statistics an exact
//! accumulator answers, one value per subpopulation (the full label-value
//! combination, as ASAPQuery's `Multiple*Accumulator`s key by). An exact
//! accumulator should score zero; scoring it anyway is what catches a merge
//! that loses order.

use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::subpopulation::{group_labels, Group};
use super::{f64_values, GroundTruth};

/// Score key for how many groups one accumulator held. `atomic_costs` reads
/// it to price memory and merge per group.
pub const GROUPS_PER_INSTANCE: &str = "groups_per_instance";

/// One statistic per group. No `Count`: as in ASAPQuery, count is `Sum` over
/// a value of 1 per sample, the same accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aggregate {
    Sum,
    Min,
    Max,
    /// Counter increase, as ASAPQuery's `IncreaseAccumulator` answers it
    /// unbounded: last minus first, plus the value before each drop (a reset).
    Increase,
    /// 1 for every group seen: the key set ASAPQuery's `DeltaSetAggregator`
    /// tracks, as one value per key so a missing key is scored. Its infinite
    /// error serializes as `null`, which `atomic-costs` skips the row for.
    Presence,
}

impl Aggregate {
    /// Fold one group's samples, in stream order.
    fn exact(self, values: &[f64]) -> f64 {
        match self {
            Aggregate::Sum => values.iter().sum(),
            Aggregate::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
            Aggregate::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Aggregate::Increase => {
                let resets: f64 = values
                    .windows(2)
                    .filter(|w| w[1] < w[0])
                    .map(|w| w[0])
                    .sum();
                values.last().unwrap_or(&0.0) - values.first().unwrap_or(&0.0) + resets
            }
            Aggregate::Presence => 1.0,
        }
    }
}

/// Why an accumulator could not answer for a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateQueryError {
    /// No sample for this group reached the accumulator.
    GroupNotFound(Group),
}

pub struct AggregateGT {
    /// Every label column: a group is the whole combination.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
    pub aggregate: Aggregate,
}

impl GroundTruth for AggregateGT {
    type Truth = BTreeMap<Group, f64>;
    /// One question per group: its value.
    type Probe = Group;
    type Answer = Result<f64, AggregateQueryError>;

    fn truth(&self, table: &GeneratedTable) -> Result<BTreeMap<Group, f64>, DataGenError> {
        let labels = group_labels(table, &self.group_columns)?;
        let values = f64_values(table.column(self.value_column)?)?;
        let mut per_group: BTreeMap<Group, Vec<f64>> = BTreeMap::new();
        for (group, &v) in labels.chunks(self.group_columns.len()).zip(&values) {
            let group: Group = group.iter().map(|l| l.to_string()).collect();
            per_group.entry(group).or_default().push(v);
        }
        Ok(per_group
            .into_iter()
            .map(|(g, vs)| (g, self.aggregate.exact(&vs)))
            .collect())
    }

    fn probes(&self, truth: &BTreeMap<Group, f64>) -> Vec<Group> {
        truth.keys().cloned().collect()
    }

    /// `relative_error` is the worst group, what a tolerance is checked
    /// against; `relative_error_mean` is over groups. A group the accumulator
    /// cannot answer for is infinitely wrong, and counted in `missing_groups`.
    fn score(
        &self,
        truth: &BTreeMap<Group, f64>,
        probes: &[Group],
        answers: &[Result<f64, AggregateQueryError>],
    ) -> BTreeMap<String, f64> {
        let missing = answers.iter().filter(|a| a.is_err()).count();
        let errors: Vec<f64> = probes
            .iter()
            .zip(answers)
            .map(|(g, answer)| {
                let &Ok(est) = answer else {
                    return f64::INFINITY;
                };
                let exact = truth[g];
                let abs_err = (est - exact).abs();
                if exact != 0.0 {
                    abs_err / exact.abs()
                } else {
                    abs_err
                }
            })
            .collect();
        let worst = errors.iter().copied().fold(0.0, f64::max);
        let mean = errors.iter().sum::<f64>() / errors.len().max(1) as f64;
        BTreeMap::from([
            ("relative_error".to_string(), worst),
            ("relative_error_mean".to_string(), mean),
            (GROUPS_PER_INSTANCE.to_string(), errors.len() as f64),
            ("missing_groups".to_string(), missing as f64),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increase_adds_the_value_before_each_reset() {
        // 5 - 1, plus 4 dropped at the reset to 2.
        assert_eq!(Aggregate::Increase.exact(&[1.0, 4.0, 2.0, 5.0]), 8.0);
    }
}
