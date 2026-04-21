//! Top-k family ground truth. Exact top-k from a HashMap
//! counter; reports precision@k and recall@k.

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::collections::HashMap;
use std::hash::Hash;

use super::GroundTruth;

pub struct TopkGT {
    pub k: usize,
}

impl<S, K> GroundTruth<S> for TopkGT
where
    K: Eq + Hash + Clone,
    S: Sketch<Item = K, Query = usize, Answer = Vec<(K, u64)>>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> serde_json::Value {
        let mut exact: HashMap<K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it.clone()).or_insert(0) += 1;
        }
        let mut exact_vec: Vec<(K, u64)> = exact.into_iter().collect();
        exact_vec.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        exact_vec.truncate(self.k);

        let est: Vec<(K, u64)> = sketch.query(self.k);
        let est_set: std::collections::HashSet<K> = est.iter().map(|(k, _)| k.clone()).collect();
        let truth_set: std::collections::HashSet<K> =
            exact_vec.iter().map(|(k, _)| k.clone()).collect();

        let tp = est_set.intersection(&truth_set).count() as f64;
        let precision = if est.is_empty() {
            0.0
        } else {
            tp / est.len() as f64
        };
        let recall = if truth_set.is_empty() {
            0.0
        } else {
            tp / truth_set.len() as f64
        };

        json!({
            "k": self.k,
            "precision_at_k": precision,
            "recall_at_k": recall,
            "true_top_k_count": truth_set.len(),
            "est_top_k_count": est.len(),
        })
    }
}
