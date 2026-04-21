//! Frequency-family ground truth (CMS, CountSketch). Exact
//! per-key count from `items`; reports L1, L2, mean + p99
//! relative error.

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::collections::HashMap;
use std::hash::Hash;

use super::GroundTruth;

pub struct FrequencyGT<K: Eq + Hash> {
    pub keys_to_probe: Vec<K>,
}

impl<S, K> GroundTruth<S> for FrequencyGT<K>
where
    K: Eq + Hash + Clone,
    S: Sketch<Item = K, Query = K, Answer = u64>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> serde_json::Value {
        // exact counts
        let mut exact: HashMap<&K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it).or_insert(0) += 1;
        }

        let probes = if self.keys_to_probe.is_empty() {
            items.to_vec()
        } else {
            self.keys_to_probe.clone()
        };

        let mut l1 = 0.0_f64;
        let mut l2 = 0.0_f64;
        let mut rel_errs: Vec<f64> = Vec::with_capacity(probes.len());

        for k in &probes {
            let est = sketch.query(k.clone()) as f64;
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

        json!({
            "l1_err": l1,
            "l2_err": l2.sqrt(),
            "relative_error_mean": mean_rel,
            "relative_error_p99": p99_rel,
            "probes": probes.len(),
        })
    }
}

pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx]
}
