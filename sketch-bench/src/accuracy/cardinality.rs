//! Cardinality-family ground truth (HLL). Exact distinct
//! count from `items`; reports relative error.

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::collections::HashSet;
use std::hash::Hash;
use std::hint::black_box;
use std::time::Instant;

use super::{Comparison, GroundTruth};

pub struct CardinalityGT;

/// Number of repeated `sketch.query(())` calls used to time
/// steady-state cardinality query throughput. A single call is
/// ~1 ns (HIP estimate / HashSet::len are O(1) field reads), well
/// inside `Instant::now()`'s 20-50 ns syscall/vDSO noise floor —
/// timing one call would report nothing but timer jitter and
/// cache state, swinging by 2-3× run-to-run. Running a tight
/// loop and dividing pulls the per-query cost out of the noise
/// and makes this row directly comparable to KLL (101 quantile
/// queries) / Frequency (one probe per heavy hitter).
const QUERY_TIMING_REPEATS: usize = 4096;

impl<S, K> GroundTruth<S> for CardinalityGT
where
    K: Eq + Hash,
    S: Sketch<Item = K, Query = (), Answer = f64>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let distinct: HashSet<&K> = items.iter().collect();
        let truth = distinct.len() as f64;

        let q_start = Instant::now();
        let mut acc: f64 = 0.0;
        for _ in 0..QUERY_TIMING_REPEATS {
            acc += black_box(sketch.query(black_box(())));
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;
        black_box(acc);

        let est = sketch.query(());
        let abs_err = (est - truth).abs();
        let rel_err = if truth > 0.0 { abs_err / truth } else { 0.0 };

        Comparison {
            json: json!({
                "truth": truth,
                "estimate": est,
                "absolute_error": abs_err,
                "relative_error": rel_err,
            }),
            queries: QUERY_TIMING_REPEATS as u64,
            query_wall_ns: q_ns,
        }
    }
}
