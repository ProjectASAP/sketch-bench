//! How many distinct values one subpopulation held.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use aqpbm_datagen::{ColumnData, DataGenError, GeneratedTable};

use super::{group_errors, group_labels, owned, write_groups, GroupKey};
use crate::accuracy::{curve, GroundTruth, GroupError};

/// Ground truth for a grouped cardinality sketch: how many distinct values one
/// subpopulation held. The population is the **subpopulation**, one entry per
/// distinct group, ranked by true distinct count.
pub struct SubpopCardinalityGT {
    /// Which label columns the subpopulation is taken over.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
}

/// Exact distinct-value counts per group, plus the ranking, the unfiltered
/// population, each group's record count and the stream's shape.
pub struct SubpopCardTruth {
    exact: HashMap<GroupKey, u64>,
    ranked: Vec<GroupKey>,
    all: Vec<GroupKey>,
    sizes: HashMap<GroupKey, u64>,
    records: u64,
    schema_width: usize,
}

/// One pass over the rows at whatever type the value column holds: a row
/// contributes its value to exactly one group, and the group's truth is the
/// size of that set; the second number is how many records it carried.
fn distinct_per_group<V: Eq + Hash>(
    labels: &[&str],
    columns: &[usize],
    values: &[V],
) -> HashMap<GroupKey, (u64, u64)> {
    let mut per_group: HashMap<Vec<&str>, (HashSet<&V>, u64)> = HashMap::new();
    for (group, value) in labels.chunks(columns.len()).zip(values) {
        match per_group.get_mut(group) {
            Some((seen, records)) => {
                seen.insert(value);
                *records += 1;
            }
            None => {
                per_group.insert(group.to_vec(), (HashSet::from([value]), 1));
            }
        }
    }
    per_group
        .into_iter()
        .map(|(group, (seen, records))| (owned(&group, columns), (seen.len() as u64, records)))
        .collect()
}

impl GroundTruth for SubpopCardinalityGT {
    type Truth = SubpopCardTruth;
    type Probe = GroupKey;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopCardTruth, DataGenError> {
        let labels = group_labels(table, &self.group_columns)?;
        let columns = &self.group_columns;
        let counted = match table.column(self.value_column)? {
            ColumnData::Int64(v) => distinct_per_group(&labels, columns, v),
            ColumnData::Unsigned64(v) => distinct_per_group(&labels, columns, v),
            ColumnData::String(v) => distinct_per_group(&labels, columns, v),
            ColumnData::Float64(v) => {
                let bits: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
                distinct_per_group(&labels, columns, &bits)
            }
        };
        let exact: HashMap<GroupKey, u64> =
            counted.iter().map(|(g, (d, _))| (g.clone(), *d)).collect();
        let sizes = counted.into_iter().map(|(g, (_, n))| (g, n)).collect();

        let mut by_count: Vec<(GroupKey, u64)> =
            exact.iter().map(|(g, c)| (g.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = curve::shuffled(&by_count);
        let ranked = by_count.into_iter().map(|(g, _)| g).collect();
        Ok(SubpopCardTruth {
            exact,
            ranked,
            all,
            sizes,
            records: table.row_num,
            schema_width: self.value_column,
        })
    }

    fn probes(&self, truth: &SubpopCardTruth) -> Vec<GroupKey> {
        curve::union_of(&truth.all, &truth.ranked, Clone::clone)
    }

    fn score(
        &self,
        truth: &SubpopCardTruth,
        probes: &[GroupKey],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&GroupKey, f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
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
        let groups = self.per_group(truth, probes, answers);
        write_groups(&groups, truth.records, truth.schema_width, &mut metrics);
        metrics
    }

    /// Relative error per group, as `are_all` averages it.
    fn per_group(
        &self,
        truth: &SubpopCardTruth,
        probes: &[GroupKey],
        answers: &[f64],
    ) -> Vec<GroupError> {
        let est: HashMap<&GroupKey, f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        group_errors(truth.exact.iter().filter(|(_, &d)| d > 0).map(|(g, &d)| {
            let err = (est.get(g).copied().unwrap_or(0.0) - d as f64).abs() / d as f64;
            (g.clone(), truth.sizes[g], err)
        }))
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
        assert_eq!(truth.exact[&vec![Some("a".to_string())]], 2);
        assert_eq!(truth.exact[&vec![Some("b".to_string())]], 1);
    }
}
