//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait, the
//! [`Comparison`] it returns, the per-statistic capability traits, and the
//! comparators. A comparator binds a *capability* ([`CardinalityOps`],
//! [`FrequencyOps`], …), not an algorithm, so it scores any implementation that
//! declares that capability. See `docs/DESIGN.md` §5.6.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::accumulator::Accumulator;
use crate::metrics::QueryCallSample;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
pub mod statistic;
pub mod subpopulation;
pub mod topk;

// The capability traits that declare which sketch answers which statistic.
pub use statistic::{
    CardinalityOps, FrequencyOps, QuantileOps, SubpopCardinalityOps, SubpopFrequencyOps,
    SubpopQuantileOps, TopKOps,
};

/// Output of a single ground-truth comparison run: named scalars plus the
/// timing of the estimate calls issued. Only the sketch's own estimate call is
/// timed, not the exact-truth build, so it means "ops/sec the sketch answers".
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// Named scalars, not an opaque JSON blob, so they fold generically through
    /// Welford. A key must state the population it is over (`are_top10` vs
    /// `are_all`) — the literature publishes both under the same word.
    pub metrics: BTreeMap<String, f64>,
    pub queries: u64,
    pub query_wall_ns: u64,
    /// Optional per-call samples, populated only when the comparator carries
    /// the `record_calls` flag (set by `approxbench sketchbench --raw-csv`). `None`
    /// otherwise, so production runs pay nothing.
    pub query_calls: Option<Vec<QueryCallSample>>,
}

/// The exact answer a sketch is scored against, and what it takes to get one
/// out of the sketch.
///
/// Four things, kept apart:
///
/// 1. [`truth`](Self::truth) computes the exact answer from the raw items.
/// 2. [`probes`](Self::probes) says what to ask the sketch, which is derived
///    from the truth and is the one thing only this statistic knows.
/// 3. [`ask`](Self::ask) puts **one** question to the sketch. It does no
///    timing: querying is an operation on the sketch, and the runner owns the
///    loop and the clock exactly as it does for insert.
/// 4. [`score`](Self::score) turns the answers into named error metrics.
///
/// Step 3 used to live in here with the other three, which is why query
/// throughput could only ever be a by-product of measuring accuracy.
pub trait GroundTruth<S: Accumulator> {
    /// The exact answer, plus whatever [`probes`](Self::probes) and
    /// [`score`](Self::score) need to read off it.
    type Truth;
    /// One question for the sketch.
    type Probe;
    /// One answer from the sketch.
    type Answer;

    /// ① The exact answer, over the whole stream.
    fn truth(&self, items: &[S::Item]) -> Self::Truth;

    /// ② What to ask, in the order the answers come back. An empty set means
    /// there is nothing to ask, and the runner measures no queries.
    fn probes(&self, truth: &Self::Truth) -> Vec<Self::Probe>;

    /// ③ One question. Nothing else: no clock, no counter, no allocation the
    /// caller did not ask for.
    fn ask(&self, sketch: &S, probe: &Self::Probe) -> Self::Answer;

    /// ④ The error metrics. A key must state the population it is over
    /// (`are_top10` vs `are_all`), since the literature publishes both under
    /// the same word.
    fn score(
        &self,
        truth: &Self::Truth,
        probes: &[Self::Probe],
        answers: &[Self::Answer],
    ) -> BTreeMap<String, f64>;
}

/// Put the whole probe set to the sketch, timing it. This is step ③, and it
/// lives here because querying is an operation on the sketch: the runner owns
/// the loop and the clock, and the comparator owns only what one question is.
///
/// `per_call` records each question's own duration. It allocates and reads the
/// clock inside the loop, so it is what a latency measurement asks for and
/// what a throughput measurement must not.
pub fn run_probes<S, G>(gt: &G, sketch: &S, items: &[S::Item], per_call: bool) -> Comparison
where
    S: Accumulator,
    G: GroundTruth<S>,
{
    let truth = gt.truth(items);
    let probes = gt.probes(&truth);

    let mut answers = Vec::with_capacity(probes.len());
    let mut calls = per_call.then(|| Vec::with_capacity(probes.len()));

    let start = Instant::now();
    for (i, probe) in probes.iter().enumerate() {
        match calls.as_mut() {
            Some(samples) => {
                let t0 = Instant::now();
                let answer = gt.ask(sketch, probe);
                let ns = t0.elapsed().as_nanos() as u64;
                samples.push(QueryCallSample {
                    call_index: i,
                    nanoseconds: ns,
                    estimate: f64::NAN,
                    percentile: f64::NAN,
                    repeat: 0,
                });
                answers.push(answer);
            }
            None => answers.push(gt.ask(sketch, probe)),
        }
    }
    let query_wall_ns = start.elapsed().as_nanos() as u64;

    Comparison {
        metrics: gt.score(&truth, &probes, &answers),
        queries: probes.len() as u64,
        query_wall_ns,
        query_calls: calls,
    }
}
