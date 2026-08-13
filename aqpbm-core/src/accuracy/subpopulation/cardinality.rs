//! Subpopulation cardinality: how many distinct values one group held. The
//! population is the group, which a counter array cannot answer.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use crate::accumulator::Accumulator;
use crate::accuracy::statistic::SubpopCardinalityOps;
use crate::accuracy::GroundTruth;
use crate::workload::Labeled;

use super::{rank_and_shuffle, score_counting_population, subset_key};

pub struct SubpopCardinalityGT {
    pub label_column: usize,
}

pub struct SubpopCardTruth {
    exact: HashMap<String, u64>,
    ranked: Vec<String>,
    all: Vec<String>,
}

impl<S, V> GroundTruth<S> for SubpopCardinalityGT
where
    V: Eq + Hash,
    S: Accumulator<Item = Labeled<V>> + SubpopCardinalityOps,
{
    type Truth = SubpopCardTruth;
    type Probe = String;
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopCardTruth {
        let mut distinct: HashMap<String, HashSet<&V>> = HashMap::new();
        for it in items {
            let Some(group) = subset_key(it, &[self.label_column]) else {
                continue;
            };
            distinct.entry(group).or_default().insert(&it.value);
        }
        let exact: HashMap<String, u64> = distinct
            .into_iter()
            .map(|(group, values)| (group, values.len() as u64))
            .collect();
        let (ranked, all) = rank_and_shuffle(exact.iter().map(|(g, c)| (g.clone(), *c)));
        SubpopCardTruth { exact, ranked, all }
    }

    fn probes(&self, truth: &SubpopCardTruth) -> Vec<String> {
        truth.all.clone()
    }

    fn ask(&self, sketch: &S, probe: &String) -> f64 {
        sketch.estimate_subpop_cardinality(&[probe.as_str()])
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    /// A member is a group, so `probes` and `subpopulations` agree here and do
    /// not for the frequency ground truth.
    fn score(
        &self,
        truth: &SubpopCardTruth,
        probes: &[String],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&String, f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let mut metrics =
            score_counting_population(&truth.ranked, &truth.all, truth.exact.len(), |g| {
                (
                    *est.get(g).unwrap_or(&0.0),
                    *truth.exact.get(g).unwrap_or(&0) as f64,
                )
            });
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}
