//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait, the
//! [`Comparison`] it returns, and the comparators.
//!
//! A comparator knows the *statistic* — how to compute the exact answer, what
//! to ask, and how to score what comes back. It does not know how to ask a
//! sketch anything; that is the row's own closure. See `docs/DESIGN.md` §5.6
//! and [`statistic`] for what used to live here.

use std::collections::BTreeMap;
use std::time::Instant;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
pub mod statistic;
pub mod subpopulation;

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
}

/// The exact answer a sketch is scored against.
///
/// Three things, kept apart:
///
/// 1. [`truth`](Self::truth) computes the exact answer from the raw items.
/// 2. [`probes`](Self::probes) says what to ask, which is derived from the
///    truth and is the one thing only this statistic knows.
/// 3. [`score`](Self::score) turns the answers into named error metrics.
///
/// **Note what is absent: putting the question to the sketch.** This trait is
/// parameterised by the *item* type, not the sketch, and never touches a
/// sketch at all. Asking is supplied per row as a closure — see `run_probes`'s
/// `ask` argument and the bodies in `sketch_bench::registry::REGISTRY`.
///
/// That split is the point. When asking lived in here, the signature had to
/// hold for every implementation at once: `&self` (so a library needing
/// `&mut self` had to be wrapped in a `RefCell`), and a probe/answer pair
/// fixed by one of seven capability traits (so a statistic none of them named
/// could not be scored at all). A closure written at the row is bound by
/// neither.
pub trait GroundTruth<I> {
    /// The exact answer, plus whatever [`probes`](Self::probes) and
    /// [`score`](Self::score) need to read off it.
    type Truth;
    /// One question for the sketch.
    type Probe;
    /// One answer from the sketch.
    type Answer;

    /// ① The exact answer, over the whole stream.
    fn truth(&self, items: &[I]) -> Self::Truth;

    /// ② What to ask, in the order the answers come back. An empty set means
    /// there is nothing to ask, and the runner measures no queries.
    fn probes(&self, truth: &Self::Truth) -> Vec<Self::Probe>;

    /// ③ The error metrics. A key must state the population it is over
    /// (`are_top10` vs `are_all`), since the literature publishes both under
    /// the same word.
    fn score(
        &self,
        truth: &Self::Truth,
        probes: &[Self::Probe],
        answers: &[Self::Answer],
    ) -> BTreeMap<String, f64>;
}

/// Put the whole probe set to the sketch, timing it.
///
/// `ask` is the row's own query body. It arrives as a closure rather than a
/// trait method so that each row states how *its* sketch is queried — including
/// taking `&mut`, which several libraries need — without every other row having
/// to agree on the signature.
///
pub fn run_probes<S, I, G, A>(gt: &G, ask: &A, sketch: &mut S, items: &[I]) -> Comparison
where
    G: GroundTruth<I>,
    A: Fn(&mut S, &G::Probe) -> G::Answer,
{
    let truth = gt.truth(items);
    let probes = gt.probes(&truth);

    let mut answers = Vec::with_capacity(probes.len());

    // One clock around the whole sweep. There is deliberately no per-call
    // timing: reading the clock inside the loop is what a query *latency*
    // measurement would need, and nothing consumes one — see `is_measurable`.
    let start = Instant::now();
    for probe in probes.iter() {
        answers.push(ask(sketch, probe));
    }
    let query_wall_ns = start.elapsed().as_nanos() as u64;

    Comparison {
        metrics: gt.score(&truth, &probes, &answers),
        queries: probes.len() as u64,
        query_wall_ns,
    }
}
