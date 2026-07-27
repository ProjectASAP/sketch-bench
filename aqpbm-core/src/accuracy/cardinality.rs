//! Cardinality-family ground truth (HLL). Exact distinct
//! count from `items`; reports relative error.

use crate::accumulator::Accumulator;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::hint::black_box;
use std::time::Instant;

use super::statistic::CardinalityOps;
use super::{Comparison, GroundTruth};
use crate::metrics::QueryCallSample;

#[derive(Debug, Default, Clone, Copy)]
pub struct CardinalityGT {
    /// When set, also stash a `Vec<QueryCallSample>` of independently-timed
    /// `estimate_distinct()` calls so the legacy
    /// `hll_throughput_query_results_rust.csv` shape (one row per call) can be
    /// emitted. Off by default — production accuracy runs pay nothing.
    pub record_calls: bool,
}

/// Number of repeated `estimate_distinct()` calls used to time
/// steady-state cardinality query throughput. The cheap estimators
/// answer from a cached field in ~1 ns, well inside `Instant::now()`'s
/// 20-50 ns syscall/vDSO noise floor — timing one call would report
/// nothing but timer jitter and cache state, swinging by 2-3× run to
/// run. Looping and dividing pulls the per-query cost out of the noise
/// and makes this row directly comparable to KLL (101 quantile queries)
/// / Frequency (one probe per heavy hitter).
const QUERY_TIMING_REPEATS: usize = 4096;

/// How many independently-timed `estimate_distinct()` calls to record
/// per run when `record_calls` is on. Matches the legacy HLL query
/// binary's `CALLS_PER_RUN = 10`.
const RAW_CALLS_PER_RUN: usize = 10;

impl<S, K> GroundTruth<S> for CardinalityGT
where
    K: Eq + Hash,
    S: Accumulator<Item = K> + CardinalityOps,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let distinct: HashSet<&K> = items.iter().collect();
        let truth = distinct.len() as f64;

        let q_start = Instant::now();
        let mut acc: f64 = 0.0;
        for _ in 0..QUERY_TIMING_REPEATS {
            acc += black_box(black_box(sketch).estimate_distinct());
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;
        black_box(acc);

        let est = sketch.estimate_distinct();
        let abs_err = (est - truth).abs();
        let rel_err = if truth > 0.0 { abs_err / truth } else { 0.0 };

        let query_calls = if self.record_calls {
            let mut samples = Vec::with_capacity(RAW_CALLS_PER_RUN);
            for i in 1..=RAW_CALLS_PER_RUN {
                let t0 = Instant::now();
                let est = black_box(black_box(sketch).estimate_distinct());
                let ns = t0.elapsed().as_nanos() as u64;
                samples.push(QueryCallSample {
                    call_index: i,
                    nanoseconds: ns,
                    estimate: est,
                    percentile: f64::NAN,
                    repeat: 0,
                });
            }
            Some(samples)
        } else {
            None
        };

        // The null estimator for cardinality answers 0, giving
        // `relative_error = 1.0` exactly — the same constant the frequency
        // comparator documents. Any implementation scoring above 1.0 is
        // worse than doing no work.
        let metrics = BTreeMap::from([
            ("truth".to_string(), truth),
            ("estimate".to_string(), est),
            ("absolute_error".to_string(), abs_err),
            ("relative_error".to_string(), rel_err),
        ]);

        Comparison {
            metrics,
            queries: QUERY_TIMING_REPEATS as u64,
            query_wall_ns: q_ns,
            query_calls,
        }
    }
}
