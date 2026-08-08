//! Frequency-algorithm ground truth (CMS, CountSketch).
//!
//! Error ships as a curve over the true top-k (`are_top1`…`are_top1000`) plus
//! `are_all`: ARE over all distinct keys is dominated by singletons, where an
//! all-zero estimator scores 1.0 (SALSA, ICDE 2021). AAE weights heavy keys.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;

use crate::accumulator::Accumulator;

use super::statistic::FrequencyOps;
use super::GroundTruth;

/// Prefix lengths of the true-frequency ranking at which error is reported.
/// Prefixes of one sorted array, so the whole curve costs one sort.
const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

#[derive(Debug, Default, Clone, Copy)]
pub struct FrequencyGT;

/// Everything the probe set and the scoring read: the exact counts, the true
/// ranking, and the unfiltered population the `*_all` keys come from.
pub struct FrequencyTruth<K> {
    exact: HashMap<K, u64>,
    /// Keys by true count descending, ties on the key, so a top-k prefix is
    /// deterministic across runs and implementations.
    ranked: Vec<K>,
    /// The `*_all` population: distinct keys shuffled then capped.
    all: Vec<K>,
}

impl<S, K> GroundTruth<S> for FrequencyGT
where
    K: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = K> + FrequencyOps<Key = K>,
{
    type Truth = FrequencyTruth<K>;
    type Probe = K;
    type Answer = u64;

    fn truth(&self, items: &[K]) -> FrequencyTruth<K> {
        let mut exact: HashMap<K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it.clone()).or_insert(0) += 1;
        }
        let mut by_count: Vec<(K, u64)> = exact.iter().map(|(k, c)| (k.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = shuffled(&by_count);
        let ranked = by_count.into_iter().map(|(k, _)| k).collect();
        FrequencyTruth { exact, ranked, all }
    }

    /// Every key any population needs, asked once. The prefixes re-use the
    /// answers instead of re-querying: an estimate is deterministic given the
    /// sketch, so a second call would only measure a warm cache.
    fn probes(&self, truth: &FrequencyTruth<K>) -> Vec<K> {
        let mut seen: std::collections::HashSet<&K> = std::collections::HashSet::new();
        let mut out: Vec<K> = Vec::with_capacity(truth.all.len());
        for k in truth.all.iter() {
            if seen.insert(k) {
                out.push(k.clone());
            }
        }
        let deepest = *TOP_K_REPORTED.last().unwrap_or(&0);
        for k in truth.ranked.iter().take(deepest) {
            if seen.insert(k) {
                out.push(k.clone());
            }
        }
        out
    }

    fn ask(&self, sketch: &S, probe: &K) -> u64 {
        sketch.estimate_frequency(probe)
    }

    fn answer_as_f64(&self, answer: &u64) -> f64 {
        *answer as f64
    }

    fn score(
        &self,
        truth: &FrequencyTruth<K>,
        probes: &[K],
        answers: &[u64],
    ) -> BTreeMap<String, f64> {
        let estimates: HashMap<&K, u64> = probes.iter().zip(answers).map(|(k, v)| (k, *v)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();

        let population = |keys: &[K], label: &str, metrics: &mut BTreeMap<String, f64>| {
            if keys.is_empty() {
                return;
            }
            let (mut are, mut aae, mut l1, mut l2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut counted = 0usize;
            for k in keys {
                let est = *estimates.get(k).unwrap_or(&0) as f64;
                let t = *truth.exact.get(k).unwrap_or(&0) as f64;
                let diff = (est - t).abs();
                l1 += diff;
                l2 += diff * diff;
                aae += diff;
                if t > 0.0 {
                    are += diff / t;
                    counted += 1;
                }
            }
            let n = keys.len() as f64;
            metrics.insert(
                format!("are_{label}"),
                if counted > 0 { are / counted as f64 } else { 0.0 },
            );
            metrics.insert(format!("aae_{label}"), aae / n);
            metrics.insert(format!("probes_{label}"), n);
            if label == "all" {
                metrics.insert("l1_err".into(), l1);
                metrics.insert("l2_err".into(), l2.sqrt());
            }
        };

        for k in TOP_K_REPORTED {
            if k > truth.ranked.len() {
                // Reporting `are_top1000` for a stream with 400 distinct keys
                // would silently mean `are_all` under a name that claims
                // otherwise. Omit it instead.
                break;
            }
            population(&truth.ranked[..k], &format!("top{k}"), &mut metrics);
        }
        population(&truth.all, "all", &mut metrics);

        // `relative_error_mean` / `_p99` / `probes` name the unfiltered
        // population; `are_all` is the same number under a name that says so.
        if let Some(v) = metrics.get("are_all").copied() {
            metrics.insert("relative_error_mean".into(), v);
        }
        if let Some(v) = metrics.get("probes_all").copied() {
            metrics.insert("probes".into(), v);
        }
        let mut errs: Vec<f64> = truth
            .all
            .iter()
            .filter_map(|k| {
                let t = *truth.exact.get(k).unwrap_or(&0) as f64;
                if t <= 0.0 {
                    return None;
                }
                let est = *estimates.get(k).unwrap_or(&0) as f64;
                Some((est - t).abs() / t)
            })
            .collect();
        errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        metrics.insert("relative_error_p99".into(), percentile(&errs, 0.99));
        metrics
    }
}

/// Every distinct key, shuffled. Shuffled so probe order does not hand the
/// baseline the locality that encounter order would, and the seed is fixed so
/// that order is reproducible. No cap and no threshold knob: scoring accuracy
/// means comparing against the truth, and a sampled population is not it.
fn shuffled<K: Clone>(ranked: &[(K, u64)]) -> Vec<K> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut out: Vec<K> = ranked.iter().map(|(k, _)| k.clone()).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
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
        let gt = FrequencyGT;
        let cmp = crate::accuracy::run_probes(&gt, &NullFreq, &items, false);
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
        let gt = FrequencyGT;
        let cmp = crate::accuracy::run_probes(&gt, &NullFreq, &items, false);
        // top1 is key 1, so AAE over it is exactly its true count.
        assert_eq!(cmp.metrics["aae_top1"], 100.0);
        assert_eq!(cmp.metrics["probes_top1"], 1.0);
        assert_eq!(cmp.metrics["probes_top10"], 10.0);
        // 200 distinct keys: top1000 must be omitted, not silently aliased
        // to `all`.
        assert!(!cmp.metrics.contains_key("are_top1000"));
    }
}
