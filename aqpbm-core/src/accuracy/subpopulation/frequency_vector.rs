use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;

use aqpbm_datagen::{ColumnData, DataGenError, GeneratedTable};

use super::{group_labels, owned, Group};
use crate::accuracy::curve;

pub struct SubpopVectorTruth {
    pub(super) exact: HashMap<Group, f64>,
    ranked: Vec<Group>,
    all: Vec<Group>,
}

fn histogram_per_group<V: Eq + Hash>(
    labels: &[&str],
    width: usize,
    values: &[V],
) -> HashMap<Group, Vec<u64>> {
    let mut per_group: HashMap<Vec<&str>, HashMap<&V, u64>> = HashMap::new();
    for (group, value) in labels.chunks(width).zip(values) {
        match per_group.get_mut(group) {
            Some(seen) => *seen.entry(value).or_insert(0) += 1,
            None => {
                per_group.insert(group.to_vec(), HashMap::from([(value, 1)]));
            }
        }
    }
    per_group
        .into_iter()
        .map(|(group, seen)| (owned(&group), seen.into_values().collect()))
        .collect()
}

pub(super) fn counts_per_group(
    table: &GeneratedTable,
    group_columns: &[usize],
    value_column: usize,
) -> Result<HashMap<Group, Vec<u64>>, DataGenError> {
    let labels = group_labels(table, group_columns)?;
    let width = group_columns.len();
    Ok(match table.column(value_column)? {
        ColumnData::Int64(v) => histogram_per_group(&labels, width, v),
        ColumnData::Unsigned64(v) => histogram_per_group(&labels, width, v),
        ColumnData::String(v) => histogram_per_group(&labels, width, v),
        ColumnData::Float64(v) => {
            let bits: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
            histogram_per_group(&labels, width, &bits)
        }
    })
}

pub(super) fn truth_over(
    table: &GeneratedTable,
    group_columns: &[usize],
    value_column: usize,
    fold: impl Fn(&[u64]) -> f64,
) -> Result<SubpopVectorTruth, DataGenError> {
    let counts = counts_per_group(table, group_columns, value_column)?;
    let exact: HashMap<Group, f64> = counts
        .iter()
        .map(|(group, counts)| (group.clone(), fold(counts)))
        .collect();

    let mut by_statistic: Vec<(Group, f64)> = exact.iter().map(|(g, v)| (g.clone(), *v)).collect();
    by_statistic.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let sized: Vec<(Group, u64)> = by_statistic
        .iter()
        .map(|(g, _)| {
            let size = counts.get(g).map(|c| c.iter().sum()).unwrap_or(0);
            (g.clone(), size)
        })
        .collect();
    let all = curve::shuffled(&sized);
    let ranked = by_statistic.into_iter().map(|(g, _)| g).collect();
    Ok(SubpopVectorTruth { exact, ranked, all })
}

pub(super) fn probes_over(truth: &SubpopVectorTruth) -> Vec<Group> {
    curve::union_of(&truth.all, &truth.ranked, Clone::clone)
}

pub(super) fn score_over(
    truth: &SubpopVectorTruth,
    probes: &[Group],
    answers: &[f64],
    group_columns: usize,
) -> BTreeMap<String, f64> {
    let est: HashMap<&Group, f64> = probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
    let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
    curve::error_curve(
        &truth.ranked,
        &truth.all,
        |g| {
            (
                *est.get(g).unwrap_or(&0.0),
                *truth.exact.get(g).unwrap_or(&0.0),
            )
        },
        &mut metrics,
    );
    metrics.insert("subpopulations".into(), truth.exact.len() as f64);
    metrics.insert("group_columns".into(), group_columns as f64);
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::records;

    fn group(label: &str) -> Group {
        vec![label.to_string()]
    }

    #[test]
    fn a_group_histogram_counts_occurrences_of_each_distinct_value() {
        let counts = counts_per_group(&records(), &[0], 2).expect("the table has the columns");
        let mut a = counts[&group("a")].clone();
        a.sort_unstable();
        assert_eq!(a, vec![1, 2]);
        assert_eq!(counts[&group("b")], vec![3]);
    }

    #[test]
    fn a_group_that_is_not_text_is_refused() {
        let Err(err) = counts_per_group(&records(), &[2], 2) else {
            panic!("an i64 grouping column must be refused, not grouped over");
        };
        let err = err.to_string();
        assert!(err.contains("i64") && err.contains("string"), "{err}");
    }
}
