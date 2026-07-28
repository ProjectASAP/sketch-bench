//! Frequency-algorithm ground truth (CMS, CountSketch).
//!
//! Error ships as a curve over the true top-k (`are_top1`…`are_top1000`) plus
//! `are_all`: ARE over all distinct keys is dominated by singletons, where an
//! all-zero estimator scores 1.0 (SALSA, ICDE 2021). AAE weights heavy keys.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use crate::accumulator::Accumulator;

use super::statistic::FrequencyOps;
use super::{Comparison, GroundTruth};

/// Prefix lengths of the true-frequency ranking at which error is reported.
/// Prefixes of one sorted array, so the whole curve costs one sort.
const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

pub struct FrequencyGT {
    /// Cap on how many distinct keys the `*_all` population probes. `0` =
    /// every distinct key. The top-k prefixes are never capped by this: they
    /// are the k heaviest keys, which is a bounded set by definition.
    pub max_probes: usize,
}

impl<S, K> GroundTruth<S> for FrequencyGT
where
    K: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = K> + FrequencyOps<Key = K>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let mut exact: HashMap<&K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it).or_insert(0) += 1;
        }

        // Rank by true count descending, ties broken on the key, so every
        // top-k prefix is deterministic across runs and implementations —
        // a HashMap's iteration order is not.
        let mut ranked: Vec<(&K, u64)> = exact.iter().map(|(k, c)| (*k, *c)).collect();
        ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        // The `*_all` population: every distinct key, shuffled so probe order
        // does not hand the baseline HashMap the locality that encounter order
        // would, then capped. Fixed seed, so the choice is reproducible.
        let all_probes = sample_distinct(&ranked, self.max_probes);

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        let mut queries = 0u64;
        let mut query_ns = 0u64;

        // Query throughput is attributed to the `all` sweep only: top-k
        // prefixes re-query keys `all` already covers, so counting them would
        // measure hot keys in L1 and move whenever `TOP_K_REPORTED` changed.
        let mut probe = |keys: &[&K], label: &str, metrics: &mut BTreeMap<String, f64>| {
            if keys.is_empty() {
                return;
            }
            let mut estimates = Vec::with_capacity(keys.len());
            let start = Instant::now();
            for k in keys {
                estimates.push(sketch.estimate_frequency(k));
            }
            if label == "all" {
                query_ns += start.elapsed().as_nanos() as u64;
                queries += keys.len() as u64;
            }

            let (mut are, mut aae, mut l1, mut l2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut counted = 0usize;
            for (k, est) in keys.iter().zip(estimates.iter()) {
                let truth = *exact.get(*k).unwrap_or(&0) as f64;
                let diff = (*est as f64 - truth).abs();
                l1 += diff;
                l2 += diff * diff;
                aae += diff;
                if truth > 0.0 {
                    are += diff / truth;
                    counted += 1;
                }
            }
            let n = keys.len() as f64;
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
                // Reporting `are_top1000` for a stream with 400 distinct keys
                // would silently mean `are_all` under a name that claims
                // otherwise. Omit it instead.
                break;
            }
            let keys: Vec<&K> = ranked[..k].iter().map(|(key, _)| *key).collect();
            probe(&keys, &format!("top{k}"), &mut metrics);
        }
        probe(&all_probes, "all", &mut metrics);

        // `relative_error_mean` / `_p99` / `probes` name the unfiltered
        // population; `are_all` is the same number under a name that says so.
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

        Comparison {
            metrics,
            queries,
            query_wall_ns: query_ns,
            query_calls: None,
        }
    }
}

/// p99 of the per-key relative error over the unfiltered population.
/// Computed in a second pass so the timed probe loop above stays a plain
/// query loop.
fn p99_relative_error<S, K>(sketch: &S, keys: &[&K], exact: &HashMap<&K, u64>) -> f64
where
    K: Eq + Hash + Clone,
    S: Accumulator<Item = K> + FrequencyOps<Key = K>,
{
    let mut errs: Vec<f64> = keys
        .iter()
        .filter_map(|k| {
            let truth = *exact.get(*k).unwrap_or(&0) as f64;
            if truth <= 0.0 {
                return None;
            }
            let est = sketch.estimate_frequency(k) as f64;
            Some((est - truth).abs() / truth)
        })
        .collect();
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile(&errs, 0.99)
}

/// The distinct keys, shuffled, then capped at `max_probes` (`0` = no cap).
/// No threshold knob: the top-k prefixes answer the same question without
/// asking an operator to pick a heavy-hitter cutoff in advance.
fn sample_distinct<'a, K>(ranked: &[(&'a K, u64)], max_probes: usize) -> Vec<&'a K> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut out: Vec<&K> = ranked.iter().map(|(k, _)| *k).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
    if max_probes != 0 && out.len() > max_probes {
        out.truncate(max_probes);
    }
    out
}

pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimator that does no work: every frequency is zero.
    struct NullFreq;
    impl Accumulator for NullFreq {
        type Item = i64;
        fn update(&mut self, _: &i64) {}
    }

    // The null estimator has to declare itself a frequency estimator like any
    // other; being shaped like one is not enough.
    impl FrequencyOps for NullFreq {
        type Key = i64;
        fn estimate_frequency(&self, _: &i64) -> u64 {
            0
        }
    }

    /// The property that makes the top-k curve worth reporting: a null
    /// estimator scores exactly 1.0 on ARE, on every population. A real sketch
    /// ranked *above* 1.0 means the metric is measuring the wrong thing.
    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let items: Vec<i64> = (0..2000).map(|i| (i % 97) as i64).collect();
        let gt = FrequencyGT { max_probes: 0 };
        let cmp = gt.compare(&NullFreq, &items);
        for key in ["are_all", "are_top1", "are_top10"] {
            let v = cmp.metrics[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        // And zero absolute error is impossible unless the data is empty:
        // AAE for the null estimator is the mean true count.
        assert!(cmp.metrics["aae_all"] > 0.0);
    }

    #[test]
    fn top_k_prefixes_follow_the_true_ranking() {
        // Key 1 appears 100x, key 2 50x, the rest once.
        let mut items: Vec<i64> = Vec::new();
        items.extend(std::iter::repeat(1).take(100));
        items.extend(std::iter::repeat(2).take(50));
        items.extend(3..=200);
        let gt = FrequencyGT { max_probes: 0 };
        let cmp = gt.compare(&NullFreq, &items);
        // top1 is key 1, so AAE over it is exactly its true count.
        assert_eq!(cmp.metrics["aae_top1"], 100.0);
        assert_eq!(cmp.metrics["probes_top1"], 1.0);
        assert_eq!(cmp.metrics["probes_top10"], 10.0);
        // 200 distinct keys: top1000 must be omitted, not silently aliased
        // to `all`.
        assert!(!cmp.metrics.contains_key("are_top1000"));
    }
}
