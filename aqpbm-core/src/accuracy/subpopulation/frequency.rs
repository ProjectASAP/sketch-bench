//! How often one value occurred inside one subpopulation.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{group_labels, owned, Group};
use crate::accuracy::{curve, CountedValue, GroundTruth};

pub struct SubpopFrequencyGT<V: CountedValue> {
    /// Which label columns the subpopulation is taken over. A grouped sketch
    /// stores every column subset, but each one is its own population with its
    /// own error, so a comparator scores one and names it.
    pub group_columns: Vec<usize>,
    pub value_column: usize,
    value: PhantomData<V>,
}

impl<V: CountedValue> SubpopFrequencyGT<V> {
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
pub struct SubpopFreqTruth<V: CountedValue> {
    exact: HashMap<(Group, V::CountKey), u64>,
    ranked: Vec<(Group, V)>,
    all: Vec<(Group, V)>,
    subpopulations: usize,
}

type CountedByGroup<'a, V> =
    HashMap<Vec<&'a str>, HashMap<<V as CountedValue>::CountKey, (u64, V)>>;

fn counted_key<V: CountedValue>(pair: &(Group, V)) -> (Group, V::CountKey) {
    (pair.0.clone(), pair.1.count_key())
}

impl<V> GroundTruth for SubpopFrequencyGT<V>
where
    V: CountedValue,
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
        let mut counted: CountedByGroup<V> = HashMap::new();
        for (group, value) in labels.chunks(width).zip(values) {
            let key = value.count_key();
            match counted.get_mut(group) {
                Some(seen) => match seen.get_mut(&key) {
                    Some(held) => held.0 += 1,
                    None => {
                        seen.insert(key, (1, value.clone()));
                    }
                },
                None => {
                    counted.insert(group.to_vec(), HashMap::from([(key, (1, value.clone()))]));
                }
            }
        }
        let subpopulations = counted.len();
        let mut exact: HashMap<(Group, V::CountKey), u64> = HashMap::new();
        let mut by_count: Vec<((Group, V), u64)> = Vec::new();
        for (group, seen) in counted {
            let group = owned(&group);
            for (key, (count, value)) in seen {
                exact.insert((group.clone(), key), count);
                by_count.push(((group.clone(), value), count));
            }
        }

        // Rank by true count descending, ties on the pair, so every top-k
        // prefix is deterministic across runs and implementations.
        by_count.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| counted_key(&a.0).cmp(&counted_key(&b.0)))
        });
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
        curve::union_of(&truth.all, &truth.ranked, counted_key)
    }

    fn score(
        &self,
        truth: &SubpopFreqTruth<V>,
        probes: &[(Group, V)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<(Group, V::CountKey), f64> = probes
            .iter()
            .zip(answers)
            .map(|(p, a)| (counted_key(p), *a))
            .collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        curve::error_curve(
            &truth.ranked,
            &truth.all,
            |pair| {
                let key = counted_key(pair);
                (
                    *est.get(&key).unwrap_or(&0.0),
                    *truth.exact.get(&key).unwrap_or(&0) as f64,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::subpopulation::records;

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
}
