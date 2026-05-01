//! Frequency-family ground truth (CMS, CountSketch). Exact
//! per-key count from `items`; reports L1, L2, mean + p99
//! relative error over the probe set.
//!
//! `min_true_count` filters the probes to **heavy hitters** —
//! keys whose true count meets the threshold. CMS / CountSketch
//! are designed to estimate frequencies of heavy hitters
//! accurately; rare keys with count = 1 get drowned by collision
//! noise and dominate any unfiltered relative-error mean. Setting
//! `min_true_count` > 0 reports the metric on the regime the
//! sketches actually optimise for. `0` disables the filter
//! (probe every distinct key — the original behaviour).

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use super::{Comparison, GroundTruth};

pub struct FrequencyGT<K: Eq + Hash> {
    pub keys_to_probe: Vec<K>,
    /// Minimum true count for a key to be probed. `0` = no
    /// filter. Typical heavy-hitter threshold: 100.
    pub min_true_count: u64,
}

impl<S, K> GroundTruth<S> for FrequencyGT<K>
where
    K: Eq + Hash + Clone,
    S: Sketch<Item = K, Query = K, Answer = u64>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        // exact counts
        let mut exact: HashMap<&K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it).or_insert(0) += 1;
        }

        let candidate_pool: Vec<K> = if self.keys_to_probe.is_empty() {
            items.to_vec()
        } else {
            self.keys_to_probe.clone()
        };

        // Apply heavy-hitter filter when requested. If the
        // filter wipes out every probe (cardinality below
        // threshold), fall back to the unfiltered set so the
        // metric is still reported instead of silently
        // returning zero probes.
        let candidates_total = candidate_pool.len();
        let probes: Vec<K> = if self.min_true_count == 0 {
            candidate_pool.clone()
        } else {
            let filtered: Vec<K> = candidate_pool
                .iter()
                .filter(|k| exact.get(k).copied().unwrap_or(0) >= self.min_true_count)
                .cloned()
                .collect();
            if filtered.is_empty() {
                candidate_pool.clone()
            } else {
                filtered
            }
        };
        let filtered_out = candidates_total - probes.len();

        // Estimate phase: time only sketch.query() so the
        // resulting query_throughput excludes ground-truth build
        // (HashMap construction).
        let mut estimates: Vec<u64> = Vec::with_capacity(probes.len());
        let q_start = Instant::now();
        for k in &probes {
            estimates.push(sketch.query(k.clone()));
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;

        let mut l1 = 0.0_f64;
        let mut l2 = 0.0_f64;
        let mut rel_errs: Vec<f64> = Vec::with_capacity(probes.len());

        for (k, est) in probes.iter().zip(estimates.iter()) {
            let est = *est as f64;
            let truth = *exact.get(k).unwrap_or(&0) as f64;
            let diff = (est - truth).abs();
            l1 += diff;
            l2 += diff * diff;
            if truth > 0.0 {
                rel_errs.push(diff / truth);
            }
        }

        rel_errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean_rel = if rel_errs.is_empty() {
            0.0
        } else {
            rel_errs.iter().sum::<f64>() / rel_errs.len() as f64
        };
        let p99_rel = percentile(&rel_errs, 0.99);

        Comparison {
            json: json!({
                "l1_err": l1,
                "l2_err": l2.sqrt(),
                "relative_error_mean": mean_rel,
                "relative_error_p99": p99_rel,
                "probes": probes.len(),
                "min_true_count": self.min_true_count,
                "filtered_out": filtered_out,
            }),
            queries: probes.len() as u64,
            query_wall_ns: q_ns,
        }
    }
}

pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx]
}
