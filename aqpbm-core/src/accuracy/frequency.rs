//! Frequency-family ground truth (CMS, CountSketch).
//!
//! ## Why the headline number is reported per top-k prefix
//!
//! The obvious metric, average relative error over every distinct key,
//! `mean(|f̂ − f| / f)`, is dominated by the keys a frequency sketch was never
//! meant to estimate. Under Zipf most distinct keys occur once or twice; a
//! collision adds a near-constant absolute overestimate, and dividing that by
//! a true count of 1 produces a huge term. Measured here, CMS `oxide` 5×2048
//! on 500k Zipf(1.1) items over 50k keys:
//!
//! ```text
//! keys with true count >= T      ARE
//!   T = 0   (all 33747)         22.31
//!   T = 10       (3396)          1.96
//!   T = 100       (383)          0.186
//!   T = 1000       (48)          0.019
//! ```
//!
//! An estimator that answers **zero for every key** scores exactly 1.0 on this
//! metric, by construction: `|0 − f| / f = 1` for every key with `f > 0`. So
//! the unfiltered 22.31 says a real Count-Min Sketch is 22× *worse* than doing
//! no work at all — and the crossover sits somewhere around a true count of
//! 10–100. That is not a defect of this implementation; it is the metric
//! measuring the wrong population, and it is a documented trap (SALSA, ICDE
//! 2021: *"for CMS, and this dataset, it is better to estimate all sizes as 0
//! without performing any measurement"*).
//!
//! So the error is reported as a **curve over the true top-k**: `are_top1`,
//! `are_top10`, `are_top100`, `are_top1000`. Where the curve crosses 1.0 is
//! itself the finding. `are_all` is still emitted — naming its population,
//! unlike the legacy `relative_error_mean` alias it duplicates — because a
//! sketch that is wildly wrong on the tail should not be able to hide it.
//!
//! ## Why ARE and AAE are both reported
//!
//! They fail in opposite directions and either one alone can be gamed:
//!
//! * `ARE = mean(|f̂ − f| / f)` — the denominator makes **rare** keys dominate.
//! * `AAE = mean(|f̂ − f|)`      — the absolute scale makes **heavy** keys dominate.
//!
//! Cormode & Hadjieleftheriou (PVLDB 2008) additionally split relative error
//! between true heavy hitters and false positives, because a single blended
//! figure hides a difference of several orders of magnitude. The top-k prefixes
//! here serve the same purpose along a continuum.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use crate::sketch::Sketch;

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
    S: Sketch<Item = K> + FrequencyOps<Key = K>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let mut exact: HashMap<&K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it).or_insert(0) += 1;
        }

        // Rank by true count, descending. Ties break on the key so the
        // ranking — and therefore every top-k prefix — is deterministic
        // across runs and across implementations; a HashMap's iteration
        // order is not.
        let mut ranked: Vec<(&K, u64)> = exact.iter().map(|(k, c)| (*k, *c)).collect();
        ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        // The `*_all` population: every distinct key, shuffled so the probe
        // order does not hand the exact-baseline HashMap the cache locality
        // that encounter order would (under Zipf the heavy hitters arrive
        // first), then capped. Fixed seed so the choice is reproducible.
        let all_probes = sample_distinct(&ranked, self.max_probes);

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        let mut queries = 0u64;
        let mut query_ns = 0u64;

        // Query throughput is attributed to the `all` sweep only. Every top-k
        // prefix re-queries keys `all` already covers — the heaviest key would
        // be probed five times — so counting them would report ops/sec over a
        // multiset that is ~40% repeated hot keys sitting in L1, and would move
        // whenever `TOP_K_REPORTED` changed. Accuracy still uses every prefix;
        // only the timing population is pinned.
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

        // `relative_error_mean` / `relative_error_p99` / `probes` keep their
        // historical names and meaning (the unfiltered population) so existing
        // plot scripts keep working. `are_all` is the same number under a name
        // that says which population it covers.
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
    S: Sketch<Item = K> + FrequencyOps<Key = K>,
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
///
/// The `min_true_count` filter this used to take is gone: it asked the
/// operator to pick a heavy-hitter threshold in advance, and its
/// empty-result fallback silently substituted a different population under
/// the same metric name. The top-k prefixes answer the same question without
/// either problem.
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
    impl Sketch for NullFreq {
        type Item = i64;
        fn update(&mut self, _: &i64) {}
        fn memory_bytes(&self) -> usize {
            0
        }
    }

    // The null estimator has to declare itself a frequency estimator like any
    // other — being shaped like one is no longer enough.
    impl FrequencyOps for NullFreq {
        type Key = i64;
        fn estimate_frequency(&self, _: &i64) -> u64 {
            0
        }
    }

    /// The property that makes the top-k curve worth reporting: a null
    /// estimator scores exactly 1.0 on ARE, on every population. Any metric
    /// that ranks a real sketch *above* 1.0 is measuring the wrong thing, and
    /// this is the constant a reader compares against.
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
