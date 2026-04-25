//! Cardinality-family ground truth (HLL). Exact distinct
//! count from `items`; reports relative error.

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::collections::HashSet;
use std::hash::Hash;
use std::time::Instant;

use super::{Comparison, GroundTruth};

pub struct CardinalityGT;

impl<S, K> GroundTruth<S> for CardinalityGT
where
    K: Eq + Hash,
    S: Sketch<Item = K, Query = (), Answer = f64>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let distinct: HashSet<&K> = items.iter().collect();
        let truth = distinct.len() as f64;
        let q_start = Instant::now();
        let est = sketch.query(());
        let q_ns = q_start.elapsed().as_nanos() as u64;
        let abs_err = (est - truth).abs();
        let rel_err = if truth > 0.0 { abs_err / truth } else { 0.0 };

        Comparison {
            json: json!({
                "truth": truth,
                "estimate": est,
                "absolute_error": abs_err,
                "relative_error": rel_err,
            }),
            queries: 1,
            query_wall_ns: q_ns,
        }
    }
}
