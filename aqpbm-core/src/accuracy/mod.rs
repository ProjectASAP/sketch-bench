//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait and the
//! comparators that implement it. A comparator knows the *statistic*, not how to
//! ask a sketch anything — that is the row's own closure, driven by `ops`.

use std::collections::BTreeMap;

pub mod cardinality;
pub(crate) mod curve;
pub mod frequency;
pub mod heavy_hitter;
pub mod quantile;
pub mod subpopulation;
pub mod topk;

/// The exact answer a sketch is scored against.
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

/// Drive one comparator end to end — truth, probes, ask, score — and hand back
/// the metric map. Test-only: production runs the same sequence inside
/// `target::Target::body`, where it sits within the region `measure` times.
#[cfg(test)]
pub(crate) fn score_with<S, I, G, A>(
    gt: &G,
    ask: &A,
    sketch: &mut S,
    items: &[I],
) -> BTreeMap<String, f64>
where
    G: GroundTruth<I>,
    A: Fn(&mut S, &G::Probe) -> G::Answer,
{
    let truth = gt.truth(items);
    let probes = gt.probes(&truth);
    let answers: Vec<G::Answer> = probes.iter().map(|p| ask(sketch, p)).collect();
    gt.score(&truth, &probes, &answers)
}
