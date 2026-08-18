//! Ground truth for the grouped sketches: a statistic taken *within* one
//! subpopulation, one comparator per statistic.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

use aqpbm_datagen::{ColumnData, ColumnItem, DataGenError, GeneratedTable};

use super::quantile::rank_err;
use super::{curve, f64_values, GroundTruth};

pub type Group = Vec<String>;

fn group_labels<'a>(
    table: &'a GeneratedTable,
    columns: &[usize],
) -> Result<Vec<&'a str>, DataGenError> {
    if columns.is_empty() {
        return Err(DataGenError::BadParam(
            "a subpopulation is taken over at least one grouping column, and none were named"
                .into(),
        ));
    }
    let grouping: Vec<&[String]> = columns
        .iter()
        .map(|&i| String::column_slice(table.column(i)?))
        .collect::<Result<Vec<_>, DataGenError>>()?;
    let rows = table.row_num as usize;
    let mut flat = Vec::with_capacity(rows * columns.len());
    for r in 0..rows {
        for column in &grouping {
            flat.push(column[r].as_str());
        }
    }
    Ok(flat)
}

fn owned(group: &[&str]) -> Group {
    group.iter().map(|label| (*label).to_string()).collect()
}

pub struct SubpopFrequencyGT<V> {
    /// Which label columns the subpopulation is taken over. A grouped sketch
    /// stores every column subset, but each one is its own population with its
    /// own error, so a comparator scores one and names it.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
    value: PhantomData<V>,
}

impl<V> SubpopFrequencyGT<V> {
    pub fn over_columns(group_columns: Vec<usize>, value_column: usize) -> Self {
        Self {
            group_columns,
            value_column,
            value: PhantomData,
        }
    }
}

/// The exact per-(group, value) counts, the ranking, and the unfiltered
/// population. Owned: the truth outlives the borrow of the table.
pub struct SubpopFreqTruth<V> {
    exact: HashMap<(Group, V), u64>,
    ranked: Vec<(Group, V)>,
    all: Vec<(Group, V)>,
    subpopulations: usize,
}

impl<V> GroundTruth for SubpopFrequencyGT<V>
where
    V: ColumnItem + Eq + Hash + Ord,
{
    type Truth = SubpopFreqTruth<V>;
    type Probe = (Group, V);
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<SubpopFreqTruth<V>, DataGenError> {
        let labels = group_labels(table, &self.group_columns)?;
        let width = self.group_columns.len();
        let values = V::column_slice(table.column(self.value_column)?)?;

        // One pass: a row contributes to exactly one (group, value) pair. The
        // group is borrowed from the table while counting and owned once at the
        // end, so a repeated group costs no allocation.
        let mut counted: HashMap<Vec<&str>, HashMap<&V, u64>> = HashMap::new();
        for (group, value) in labels.chunks(width).zip(values) {
            match counted.get_mut(group) {
                Some(values) => *values.entry(value).or_insert(0) += 1,
                None => {
                    counted.insert(group.to_vec(), HashMap::from([(value, 1)]));
                }
            }
        }
        let subpopulations = counted.len();
        let mut exact: HashMap<(Group, V), u64> = HashMap::new();
        for (group, values) in counted {
            let group = owned(&group);
            for (value, count) in values {
                exact.insert((group.clone(), value.clone()), count);
            }
        }

        // Rank by true count descending, ties on the pair, so every top-k
        // prefix is deterministic across runs and implementations.
        let mut by_count: Vec<((Group, V), u64)> =
            exact.iter().map(|(k, c)| (k.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = curve::shuffled(&by_count);
        let ranked = by_count.into_iter().map(|(k, _)| k).collect();
        Ok(SubpopFreqTruth {
            exact,
            ranked,
            all,
            subpopulations,
        })
    }

    fn probes(&self, truth: &SubpopFreqTruth<V>) -> Vec<(Group, V)> {
        curve::union_of(&truth.all, &truth.ranked)
    }

    fn score(
        &self,
        truth: &SubpopFreqTruth<V>,
        probes: &[(Group, V)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&(Group, V), f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        curve::error_curve(
            &truth.ranked,
            &truth.all,
            |pair| {
                (
                    *est.get(pair).unwrap_or(&0.0),
                    *truth.exact.get(pair).unwrap_or(&0) as f64,
                )
            },
            &mut metrics,
        );

        // How many distinct groups the grouping columns carried. A grouped
        // sketch's error is a function of this, so a record without it cannot
        // be read.
        metrics.insert("subpopulations".into(), truth.subpopulations as f64);
        metrics.insert("group_columns".into(), self.group_columns.len() as f64);
        metrics
    }
}

// ---------- subpopulation cardinality ----------

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
        curve::union_of(&truth.all, &truth.ranked)
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

// ---------- subpopulation quantile ----------

/// Number of quantiles probed per group — the same grid the ungrouped
/// comparator uses, named once in [`super::quantile`].
use super::quantile::GRID_POINTS as GROUP_GRID_POINTS;

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
    per_group: HashMap<Group, Vec<f64>>,
    /// Every group, shuffled: probe order must not hand the baseline the
    /// locality the encounter order would.
    probed: Vec<Group>,
    items: usize,
}

impl GroundTruth for SubpopRankErrorGT {
    type Truth = SubpopRankTruth;
    /// One (group, fraction) question.
    type Probe = (Group, f64);
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
        let mut per_group: HashMap<Group, Vec<f64>> = HashMap::with_capacity(borrowed.len());
        for (group, mut held) in borrowed {
            held.sort_by(f64::total_cmp);
            per_group.insert(owned(&group), held);
        }

        let mut by_size: Vec<(Group, u64)> = per_group
            .iter()
            .map(|(g, v)| (g.clone(), v.len() as u64))
            .collect();
        by_size.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let probed = curve::shuffled(&by_size);
        Ok(SubpopRankTruth {
            per_group,
            probed,
            items: table.row_num as usize,
        })
    }

    fn probes(&self, truth: &SubpopRankTruth) -> Vec<(Group, f64)> {
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
        probes: &[(Group, f64)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let mut sum_mean = 0.0f64;
        let mut max_err = 0.0f64;
        let mut scored_groups = 0usize;

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
            sum_mean += group_sum / GROUP_GRID_POINTS as f64;
            scored_groups += 1;
        }

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        metrics.insert(
            "mean_rank_err".into(),
            if scored_groups > 0 {
                sum_mean / scored_groups as f64
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
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;

    /// The estimator that does no work: every subpopulation frequency is zero.
    ///
    /// It has no `update`: the comparator computes its truth from the table,
    /// and this double answers 0 whatever it was fed, so nothing needs to go
    /// into it.
    struct NullSubpop;
    impl NullSubpop {
        fn estimate_subpop_frequency(&self, _: &[&str], _: &i64) -> f64 {
            0.0
        }
    }
    fn query_null(s: &mut NullSubpop, p: &(Group, i64)) -> f64 {
        s.estimate_subpop_frequency(&labels(&p.0), &p.1)
    }

    fn labels(group: &Group) -> Vec<&str> {
        group.iter().map(String::as_str).collect()
    }

    /// The estimator that is exactly right, built from the same rows the
    /// comparator reads. Pins that the comparator's own truth is self-consistent.
    struct ExactSubpop {
        counts: HashMap<(Group, i64), u64>,
    }
    impl ExactSubpop {
        fn estimate_subpop_frequency(&self, group: &[&str], value: &i64) -> f64 {
            *self.counts.get(&(owned(group), *value)).unwrap_or(&0) as f64
        }
    }
    fn query_exact(s: &mut ExactSubpop, p: &(Group, i64)) -> f64 {
        s.estimate_subpop_frequency(&labels(&p.0), &p.1)
    }

    /// key1 ∈ {a, b}, key2 ∈ {x, y}; the value column is what gets counted.
    fn records() -> GeneratedTable {
        let rows = [
            ("a", "x", 10),
            ("a", "y", 10),
            ("a", "x", 20),
            ("b", "x", 30),
            ("b", "y", 30),
            ("b", "y", 30),
        ];
        table_of(
            &["key1", "key2", "value"],
            vec![
                ColumnData::String(rows.iter().map(|r| r.0.to_string()).collect()),
                ColumnData::String(rows.iter().map(|r| r.1.to_string()).collect()),
                ColumnData::Int64(rows.iter().map(|r| r.2).collect()),
            ],
        )
    }

    fn over(group_columns: Vec<usize>) -> SubpopFrequencyGT<i64> {
        SubpopFrequencyGT::over_columns(group_columns, 2)
    }

    /// The property that makes ARE worth reporting: a null estimator scores
    /// exactly 1.0, on every population.
    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let cmp =
            crate::accuracy::score_with(&over(vec![0]), &query_null, &mut NullSubpop, &records());
        for key in ["are_all", "are_top1"] {
            let v = cmp[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        assert!(cmp["aae_all"] > 0.0);
    }

    /// An exact estimator scores exactly 0.0. Together with the null case this
    /// pins both ends of the scale, so a sketch landing outside them means the
    /// comparator is measuring the wrong thing.
    #[test]
    fn exact_estimator_scores_zero() {
        let table = records();
        let gt = over(vec![0]);
        let truth = gt.truth(&table).expect("the table has the named columns");
        let mut exact = ExactSubpop {
            counts: truth.exact.clone(),
        };
        let cmp = crate::accuracy::score_with(&gt, &query_exact, &mut exact, &table);
        assert_eq!(cmp["are_all"], 0.0);
        assert_eq!(cmp["aae_all"], 0.0);
        assert_eq!(cmp["l1_err"], 0.0);
    }

    /// The population is (subpopulation, value) pairs, and the grouping really
    /// is by the named column: `b;x` and `b;y` both carry value 30, so grouping
    /// by column 0 pools all three of those rows into one pair.
    #[test]
    fn truth_groups_by_the_named_column() {
        let cmp =
            crate::accuracy::score_with(&over(vec![0]), &query_null, &mut NullSubpop, &records());
        // Distinct pairs at column 0: (a,10) (a,20) (b,30) → 3 pairs, 2 groups.
        assert_eq!(cmp["probes_all"], 3.0);
        assert_eq!(cmp["subpopulations"], 2.0);
        // The heaviest pair is (b, 30) with 3 rows, so a null estimator's
        // AAE over the top-1 prefix is exactly that count.
        assert_eq!(cmp["aae_top1"], 3.0);
        // 3 distinct pairs: the top-10 prefix must be omitted, not aliased.
        assert!(!cmp.contains_key("are_top10"));
    }

    /// Scoring a different column is a different population, not a relabelling.
    #[test]
    fn a_different_column_is_a_different_population() {
        let by_col1 =
            crate::accuracy::score_with(&over(vec![1]), &query_null, &mut NullSubpop, &records());
        // Column 1 pairs: (x,10) (y,10) (x,20) (x,30) (y,30) → 5 pairs, 2 groups.
        assert_eq!(by_col1["probes_all"], 5.0);
        assert_eq!(by_col1["subpopulations"], 2.0);
        assert_eq!(by_col1["group_columns"], 1.0);
    }

    #[test]
    fn two_columns_group_on_the_pair() {
        let table = records();
        let gt = over(vec![0, 1]);
        let truth = gt.truth(&table).expect("the table has the named columns");
        assert_eq!(truth.subpopulations, 4);
        for (group, _) in truth.exact.keys() {
            assert_eq!(group.len(), 2, "one label per grouping column: {group:?}");
        }
        let cmp = crate::accuracy::score_with(&gt, &query_null, &mut NullSubpop, &table);
        // (a,x,10) (a,y,10) (a,x,20) (b,x,30) (b,y,30) → 5 pairs.
        assert_eq!(cmp["probes_all"], 5.0);
        assert_eq!(cmp["group_columns"], 2.0);
    }

    #[test]
    fn a_grouping_column_that_is_not_text_is_refused() {
        let Err(err) = over(vec![2]).truth(&records()) else {
            panic!("an i64 grouping column must be refused, not grouped over");
        };
        let err = err.to_string();
        assert!(err.contains("i64") && err.contains("string"), "{err}");
    }

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
            truth.per_group[&vec!["a".to_string()]],
            vec![10.0, 10.0, 20.0]
        );
        assert_eq!(
            truth.per_group[&vec!["b".to_string()]],
            vec![30.0, 30.0, 30.0]
        );
    }
}
