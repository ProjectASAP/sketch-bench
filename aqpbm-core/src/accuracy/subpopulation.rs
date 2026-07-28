//! Ground truth for a grouped frequency sketch: the frequency of a value
//! *within* one subpopulation.
//!
//! A record stream carries `d` label columns and one value. The population
//! scored here is the set of **(label, value) pairs** observed at one column,
//! which is what a grouped counter array actually stores: the counters live
//! inside a group and count values, so the size of the group is a different
//! statistic and not this one.
//!
//! Error ships in the same vocabulary as the ungrouped frequency comparator
//! (`are_top1`…`are_top1000`, `are_all`, `aae_*`), so a grouped row's numbers
//! land in the columns an ungrouped row already fills.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use crate::accumulator::Accumulator;
use crate::workload::Labeled;

use super::frequency::percentile;
use super::statistic::SubpopFrequencyOps;
use super::{Comparison, GroundTruth};

/// Prefix lengths of the true-frequency ranking at which error is reported.
const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

pub struct SubpopFrequencyGT {
    /// Which label column the subpopulation is taken over. A grouped sketch
    /// stores every column subset, but each one is its own population with its
    /// own error, so a comparator scores one and names it.
    pub label_column: usize,
    /// Cap on how many distinct pairs the `all` population probes. `0` = every
    /// observed pair.
    pub max_probes: usize,
}

impl<S, V> GroundTruth<S> for SubpopFrequencyGT
where
    V: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = Labeled<V>> + SubpopFrequencyOps<Value = V>,
{
    fn compare(&self, sketch: &S, items: &[Labeled<V>]) -> Comparison {
        // The whole ground truth, in one pass: a record contributes to exactly
        // one (label, value) pair at this column.
        let mut exact: HashMap<(&str, &V), u64> = HashMap::new();
        for it in items {
            let Some(label) = it.label(self.label_column) else {
                continue;
            };
            *exact.entry((label, &it.value)).or_insert(0) += 1;
        }

        // Rank by true count descending, ties broken on the pair, so every
        // top-k prefix is deterministic across runs and implementations.
        let mut ranked: Vec<((&str, &V), u64)> = exact.iter().map(|(k, c)| (*k, *c)).collect();
        ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let all_probes = sample_distinct(&ranked, self.max_probes);

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        let mut queries = 0u64;
        let mut query_ns = 0u64;

        // Query throughput is attributed to the `all` sweep only: the top-k
        // prefixes re-query pairs `all` already covers.
        let mut probe = |pairs: &[(&str, &V)], label: &str, metrics: &mut BTreeMap<String, f64>| {
            if pairs.is_empty() {
                return;
            }
            let mut estimates = Vec::with_capacity(pairs.len());
            let start = Instant::now();
            for (group, value) in pairs {
                estimates.push(sketch.estimate_subpop_frequency(&[group], value));
            }
            if label == "all" {
                query_ns += start.elapsed().as_nanos() as u64;
                queries += pairs.len() as u64;
            }

            let (mut are, mut aae, mut l1, mut l2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut counted = 0usize;
            for (pair, est) in pairs.iter().zip(estimates.iter()) {
                let truth = *exact.get(pair).unwrap_or(&0) as f64;
                let diff = (*est - truth).abs();
                l1 += diff;
                l2 += diff * diff;
                aae += diff;
                if truth > 0.0 {
                    are += diff / truth;
                    counted += 1;
                }
            }
            let n = pairs.len() as f64;
            metrics.insert(
                format!("are_{label}"),
                if counted > 0 {
                    are / counted as f64
                } else {
                    0.0
                },
            );
            metrics.insert(format!("aae_{label}"), aae / n);
            metrics.insert(format!("probes_{label}"), n);
            if label == "all" {
                metrics.insert("l1_err".into(), l1);
                metrics.insert("l2_err".into(), l2.sqrt());
            }
        };

        for k in TOP_K_REPORTED {
            if k > ranked.len() {
                // Reporting `are_top1000` over 400 distinct pairs would mean
                // `are_all` under a name claiming otherwise. Omit it.
                break;
            }
            let pairs: Vec<(&str, &V)> = ranked[..k].iter().map(|(pair, _)| *pair).collect();
            probe(&pairs, &format!("top{k}"), &mut metrics);
        }
        probe(&all_probes, "all", &mut metrics);

        if let Some(v) = metrics.get("are_all").copied() {
            metrics.insert("relative_error_mean".into(), v);
        }
        if let Some(v) = metrics.get("probes_all").copied() {
            metrics.insert("probes".into(), v);
        }
        metrics.insert(
            "relative_error_p99".into(),
            p99_relative_error(sketch, &all_probes, &exact),
        );

        // How many distinct groups the column carried. A grouped sketch's error
        // is a function of this, so a record without it cannot be read.
        let groups: std::collections::BTreeSet<&str> =
            exact.keys().map(|(group, _)| *group).collect();
        metrics.insert("subpopulations".into(), groups.len() as f64);
        metrics.insert("label_column".into(), self.label_column as f64);

        Comparison {
            metrics,
            queries,
            query_wall_ns: query_ns,
            query_calls: None,
        }
    }
}

/// p99 of the per-pair relative error over the unfiltered population.
fn p99_relative_error<S, V>(
    sketch: &S,
    pairs: &[(&str, &V)],
    exact: &HashMap<(&str, &V), u64>,
) -> f64
where
    V: Eq + Hash + Clone,
    S: Accumulator<Item = Labeled<V>> + SubpopFrequencyOps<Value = V>,
{
    let mut errs: Vec<f64> = pairs
        .iter()
        .filter_map(|pair| {
            let truth = *exact.get(pair).unwrap_or(&0) as f64;
            if truth <= 0.0 {
                return None;
            }
            let est = sketch.estimate_subpop_frequency(&[pair.0], pair.1);
            Some((est - truth).abs() / truth)
        })
        .collect();
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile(&errs, 0.99)
}

/// The distinct pairs, shuffled, then capped at `max_probes` (`0` = no cap).
/// Shuffled so probe order does not hand the baseline the locality that
/// encounter order would. Fixed seed, so the choice is reproducible.
fn sample_distinct<'a, V>(
    ranked: &[((&'a str, &'a V), u64)],
    max_probes: usize,
) -> Vec<(&'a str, &'a V)> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut out: Vec<(&str, &V)> = ranked.iter().map(|(pair, _)| *pair).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
    if max_probes != 0 && out.len() > max_probes {
        out.truncate(max_probes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimator that does no work: every subpopulation frequency is zero.
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

    /// The estimator that is exactly right, built from the same pass the
    /// comparator makes. Pins that the comparator's own truth is self-consistent.
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
        fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
            *self
                .counts
                .get(&(labels[0].to_string(), *value))
                .unwrap_or(&0) as f64
        }
    }

    fn records() -> Vec<Labeled<i64>> {
        // key1 ∈ {a, b}, key2 ∈ {x, y}; the value is what gets counted.
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

    /// The property that makes ARE worth reporting: a null estimator scores
    /// exactly 1.0, on every population.
    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let gt = SubpopFrequencyGT {
            label_column: 0,
            max_probes: 0,
        };
        let cmp = gt.compare(&NullSubpop, &records());
        for key in ["are_all", "are_top1"] {
            let v = cmp.metrics[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        assert!(cmp.metrics["aae_all"] > 0.0);
    }

    /// An exact estimator scores exactly 0.0. Together with the null case this
    /// pins both ends of the scale, so a sketch landing outside them means the
    /// comparator is measuring the wrong thing.
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
        let gt = SubpopFrequencyGT {
            label_column: 0,
            max_probes: 0,
        };
        let cmp = gt.compare(&exact, &items);
        assert_eq!(cmp.metrics["are_all"], 0.0);
        assert_eq!(cmp.metrics["aae_all"], 0.0);
        assert_eq!(cmp.metrics["l1_err"], 0.0);
    }

    /// The population is (subpopulation, value) pairs, and the grouping really
    /// is by the named column: `b;x` and `b;y` both carry value 30, so grouping
    /// by column 0 pools all three of those records into one pair.
    #[test]
    fn truth_groups_by_the_named_column() {
        let items = records();
        let gt = SubpopFrequencyGT {
            label_column: 0,
            max_probes: 0,
        };
        let cmp = gt.compare(&NullSubpop, &items);
        // Distinct pairs at column 0: (a,10) (a,20) (b,30) → 3 pairs, 2 groups.
        assert_eq!(cmp.metrics["probes_all"], 3.0);
        assert_eq!(cmp.metrics["subpopulations"], 2.0);
        // The heaviest pair is (b, 30) with 3 records, so a null estimator's
        // AAE over the top-1 prefix is exactly that count.
        assert_eq!(cmp.metrics["aae_top1"], 3.0);
        // 3 distinct pairs: the top-10 prefix must be omitted, not aliased.
        assert!(!cmp.metrics.contains_key("are_top10"));
    }

    /// Scoring a different column is a different population, not a relabelling.
    #[test]
    fn a_different_column_is_a_different_population() {
        let items = records();
        let by_col1 = SubpopFrequencyGT {
            label_column: 1,
            max_probes: 0,
        }
        .compare(&NullSubpop, &items);
        // Column 1 pairs: (x,10) (y,10) (x,20) (x,30) (y,30) → 5 pairs, 2 groups.
        assert_eq!(by_col1.metrics["probes_all"], 5.0);
        assert_eq!(by_col1.metrics["subpopulations"], 2.0);
        assert_eq!(by_col1.metrics["label_column"], 1.0);
    }
}
