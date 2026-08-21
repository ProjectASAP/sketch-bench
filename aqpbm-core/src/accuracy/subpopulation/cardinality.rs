//! How many distinct values one subpopulation held.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use aqpbm_datagen::{ColumnData, DataGenError, GeneratedTable};

use super::{group_labels, owned, Group};
use crate::accuracy::{curve, GroundTruth};

/// Ground truth for a grouped cardinality sketch: how many distinct values one
/// subpopulation held. The population is the **subpopulation**, one entry per
/// distinct group, ranked by true distinct count.
pub struct SubpopCardinalityGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

/// Exact distinct-value counts per group, plus the ranking and the unfiltered
/// population.
pub struct SubpopCardTruth {
    exact: HashMap<Group, u64>,
    ranked: Vec<Group>,
    all: Vec<Group>,
}

/// One pass over the rows at whatever type the value column holds: a row
/// contributes its value to exactly one group, and the group's truth is the
/// size of that set.
fn distinct_per_group<V: Eq + Hash>(
    labels: &[&str],
    width: usize,
    values: &[V],
) -> HashMap<Group, u64> {
    let mut per_group: HashMap<Vec<&str>, HashSet<&V>> = HashMap::new();
    for (group, value) in labels.chunks(width).zip(values) {
        match per_group.get_mut(group) {
            Some(seen) => {
                seen.insert(value);
            }
            None => {
                per_group.insert(group.to_vec(), HashSet::from([value]));
            }
        }
    }
    per_group
        .into_iter()
        .map(|(group, seen)| (owned(&group), seen.len() as u64))
        .collect()
}

impl GroundTruth for SubpopCardinalityGT {
    type Truth = SubpopCardTruth;
    type Probe = Group;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopCardTruth, DataGenError> {
        let labels = group_labels(table, &self.group_columns)?;
        let width = self.group_columns.len();
        let exact = match table.column(self.value_column)? {
            ColumnData::Int64(v) => distinct_per_group(&labels, width, v),
            ColumnData::Unsigned64(v) => distinct_per_group(&labels, width, v),
            ColumnData::String(v) => distinct_per_group(&labels, width, v),
            ColumnData::Float64(v) => {
                let bits: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
                distinct_per_group(&labels, width, &bits)
            }
        };

        let mut by_count: Vec<(Group, u64)> = exact.iter().map(|(g, c)| (g.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = curve::shuffled(&by_count);
        let ranked = by_count.into_iter().map(|(g, _)| g).collect();
        Ok(SubpopCardTruth { exact, ranked, all })
    }

    fn probes(&self, truth: &SubpopCardTruth) -> Vec<Group> {
        curve::union_of(&truth.all, &truth.ranked, Clone::clone)
    }

    fn score(
        &self,
        truth: &SubpopCardTruth,
        probes: &[Group],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&Group, f64> = probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        curve::error_curve(
            &truth.ranked,
            &truth.all,
            |g| {
                (
                    *est.get(g).unwrap_or(&0.0),
                    *truth.exact.get(g).unwrap_or(&0) as f64,
                )
            },
            &mut metrics,
        );
        metrics.insert("subpopulations".into(), truth.exact.len() as f64);
        metrics.insert("group_columns".into(), self.group_columns.len() as f64);
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::records;

    #[test]
    fn cardinality_counts_distinct_values_per_group() {
        let gt = SubpopCardinalityGT {
            group_columns: vec![0],
            value_column: 2,
        };
        let truth = gt
            .truth(&records())
            .expect("the table has the named columns");
        assert_eq!(truth.exact[&vec!["a".to_string()]], 2);
        assert_eq!(truth.exact[&vec!["b".to_string()]], 1);
    }
}
