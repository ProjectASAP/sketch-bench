//! Subpopulation frequency: how many times a value occurred inside one group.
//! The population is a (group, value) pair, not a group.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use crate::accumulator::Accumulator;
use crate::accuracy::statistic::SubpopFrequencyOps;
use crate::accuracy::GroundTruth;
use crate::workload::Labeled;

use super::{rank_and_shuffle, score_counting_population, subset_key};

pub struct SubpopFrequencyGT {
    pub label_column: usize,
}

pub struct SubpopFreqTruth<V> {
    exact: HashMap<(String, V), u64>,
    ranked: Vec<(String, V)>,
    all: Vec<(String, V)>,
}

impl<S, V> GroundTruth<S> for SubpopFrequencyGT
where
    V: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = Labeled<V>> + SubpopFrequencyOps<Value = V>,
{
    type Truth = SubpopFreqTruth<V>;
    type Probe = (String, V);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopFreqTruth<V> {
        let mut exact: HashMap<(String, V), u64> = HashMap::new();
        for it in items {
            let Some(group) = subset_key(it, &[self.label_column]) else {
                continue;
            };
            *exact.entry((group, it.value.clone())).or_insert(0) += 1;
        }
        let (ranked, all) = rank_and_shuffle(exact.iter().map(|(k, c)| (k.clone(), *c)));
        SubpopFreqTruth { exact, ranked, all }
    }

    /// One sweep answers every population: the top-k prefixes are prefixes of a
    /// permutation of `all`.
    fn probes(&self, truth: &SubpopFreqTruth<V>) -> Vec<(String, V)> {
        truth.all.clone()
    }

    fn ask(&self, sketch: &S, probe: &(String, V)) -> f64 {
        sketch.estimate_subpop_frequency(&[probe.0.as_str()], &probe.1)
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(
        &self,
        truth: &SubpopFreqTruth<V>,
        probes: &[(String, V)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&(String, V), f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let groups: HashSet<&str> = truth.exact.keys().map(|(g, _)| g.as_str()).collect();
        let mut metrics = score_counting_population(&truth.ranked, &truth.all, groups.len(), |pair| {
            (
                *est.get(pair).unwrap_or(&0.0),
                *truth.exact.get(pair).unwrap_or(&0) as f64,
            )
        });
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullSubpop;
    impl Accumulator for NullSubpop {
        type Item = Labeled<i64>;
        fn update(&mut self, _: &Labeled<i64>) {}
    }
    impl SubpopFrequencyOps for NullSubpop {
        type Value = i64;
        fn estimate_subpop_frequency(&self, _: &[&str], _: &i64) -> f64 {
            0.0
        }
    }

    struct ExactSubpop {
        counts: HashMap<(String, i64), u64>,
        column: usize,
    }
    impl Accumulator for ExactSubpop {
        type Item = Labeled<i64>;
        fn update(&mut self, r: &Labeled<i64>) {
            let label = r.label(self.column).unwrap_or("").to_string();
            *self.counts.entry((label, r.value)).or_insert(0) += 1;
        }
    }
    impl SubpopFrequencyOps for ExactSubpop {
        type Value = i64;
        /// Joined, not `labels[0]`: the query names a subset, and the key a
        /// grouped sketch files it under is the join.
        fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
            *self.counts.get(&(labels.join(";"), *value)).unwrap_or(&0) as f64
        }
    }

    /// key1 ∈ {a, b}, key2 ∈ {x, y}.
    fn records() -> Vec<Labeled<i64>> {
        [
            ("a;x", 10),
            ("a;y", 10),
            ("a;x", 20),
            ("b;x", 30),
            ("b;y", 30),
            ("b;y", 30),
        ]
        .iter()
        .map(|(k, v)| Labeled {
            key: (*k).to_string(),
            value: *v,
        })
        .collect()
    }

    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &NullSubpop, &records(), false);
        for key in ["are_all", "are_top1"] {
            let v = cmp.metrics[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        assert!(cmp.metrics["aae_all"] > 0.0);
    }

    /// With the null case above, pins both ends of the scale.
    #[test]
    fn exact_estimator_scores_zero() {
        let items = records();
        let mut exact = ExactSubpop {
            counts: HashMap::new(),
            column: 0,
        };
        for it in &items {
            exact.update(it);
        }
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &exact, &items, false);
        assert_eq!(cmp.metrics["are_all"], 0.0);
        assert_eq!(cmp.metrics["aae_all"], 0.0);
        assert_eq!(cmp.metrics["l1_err"], 0.0);
    }

    #[test]
    fn truth_groups_by_the_named_column() {
        let items = records();
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &NullSubpop, &items, false);
        // Column 0: (a,10) (a,20) (b,30) → 3 pairs, 2 groups. `b;x` and `b;y`
        // both carry 30, so all three of those records pool into one pair.
        assert_eq!(cmp.metrics["probes_all"], 3.0);
        assert_eq!(cmp.metrics["subpopulations"], 2.0);
        assert_eq!(cmp.metrics["aae_top1"], 3.0);
        // 3 pairs: the top-10 prefix must be omitted, not aliased.
        assert!(!cmp.metrics.contains_key("are_top10"));
    }

    #[test]
    fn a_different_column_is_a_different_population() {
        let items = records();
        let by_col1 = crate::accuracy::run_probes(
            &SubpopFrequencyGT { label_column: 1 },
            &NullSubpop,
            &items,
            false,
        );
        // Column 1: (x,10) (y,10) (x,20) (x,30) (y,30) → 5 pairs, 2 groups.
        assert_eq!(by_col1.metrics["probes_all"], 5.0);
        assert_eq!(by_col1.metrics["subpopulations"], 2.0);
        assert_eq!(by_col1.metrics["label_column"], 1.0);
    }
}
