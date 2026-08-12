//! Subpopulation quantile: the ordered statistic inside one group, in
//! rank-error units.
//!
//! Costs `groups * 101` estimate calls, the most of any ground truth here. A
//! run that is too slow wants fewer groups in the workload, not a sample of
//! them — a sampled population is not the truth.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::accumulator::Accumulator;
use crate::accuracy::quantile::{lower_bound, upper_bound, QuantileValue};
use crate::accuracy::statistic::SubpopQuantileOps;
use crate::accuracy::GroundTruth;
use crate::workload::Labeled;

use super::{rank_and_shuffle, subset_key};

/// The same grid [`RankErrorGT`](crate::accuracy::quantile::RankErrorGT) uses,
/// so grouped and ungrouped rank error are read on one ruler.
const GROUP_GRID_POINTS: usize = 101;

pub struct SubpopRankErrorGT {
    pub label_column: usize,
}

pub struct SubpopRankTruth {
    per_group: HashMap<String, Vec<f64>>,
    probed: Vec<String>,
    /// Records in the workload, not in the scored groups; the two differ only
    /// when a record has no label at the scored column.
    items: usize,
}

impl<S, V> GroundTruth<S> for SubpopRankErrorGT
where
    V: QuantileValue,
    S: Accumulator<Item = Labeled<V>> + SubpopQuantileOps,
{
    type Truth = SubpopRankTruth;
    type Probe = (String, f64);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopRankTruth {
        // Not deduplicated: rank is over occurrences.
        let mut per_group: HashMap<String, Vec<f64>> = HashMap::new();
        for it in items {
            let Some(group) = subset_key(it, &[self.label_column]) else {
                continue;
            };
            per_group.entry(group).or_default().push(it.value.to_f64());
        }
        for values in per_group.values_mut() {
            values.sort_by(f64::total_cmp);
        }
        // Only the probe order is kept: this reports no top-k prefix.
        let (_, probed) = rank_and_shuffle(per_group.iter().map(|(g, v)| (g.clone(), v.len() as u64)));
        SubpopRankTruth {
            per_group,
            probed,
            items: items.len(),
        }
    }

    fn probes(&self, truth: &SubpopRankTruth) -> Vec<(String, f64)> {
        let mut out = Vec::with_capacity(truth.probed.len() * GROUP_GRID_POINTS);
        for group in &truth.probed {
            match truth.per_group.get(group) {
                Some(sorted) if !sorted.is_empty() => {}
                _ => continue,
            }
            for i in 0..GROUP_GRID_POINTS {
                out.push((group.clone(), i as f64 / 100.0));
            }
        }
        out
    }

    fn ask(&self, sketch: &S, probe: &(String, f64)) -> f64 {
        sketch.estimate_subpop_quantile(&[probe.0.as_str()], probe.1)
    }

    fn probe_as_f64(&self, probe: &(String, f64)) -> f64 {
        probe.1
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(
        &self,
        truth: &SubpopRankTruth,
        probes: &[(String, f64)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let mut sum_mean = 0.0f64;
        let mut max_err = 0.0f64;
        let mut scored_groups = 0usize;

        // The probe set is one contiguous grid per group, in order.
        for (chunk_p, chunk_a) in probes
            .chunks(GROUP_GRID_POINTS)
            .zip(answers.chunks(GROUP_GRID_POINTS))
        {
            let Some((group, _)) = chunk_p.first() else {
                continue;
            };
            let Some(sorted) = truth.per_group.get(group) else {
                continue;
            };
            let nf = sorted.len() as f64;
            let mut group_sum = 0.0f64;
            for ((_, q), est) in chunk_p.iter().zip(chunk_a) {
                // The returned value occupies `[lower, upper]`, so a target
                // inside that interval is not an error.
                let target = q * nf;
                let lower = lower_bound(sorted, *est) as f64;
                let upper = upper_bound(sorted, *est) as f64;
                let raw = if target < lower {
                    lower - target
                } else if target > upper {
                    target - upper
                } else {
                    0.0
                };
                let err = raw / nf;
                group_sum += err;
                if err > max_err {
                    max_err = err;
                }
            }
            // The 101 points are one member, so they mean into that member's
            // error before the members are meaned.
            sum_mean += group_sum / GROUP_GRID_POINTS as f64;
            scored_groups += 1;
        }

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        // Unweighted, as the counting ground truths are: a 40000-record group
        // and a 125-record one each count once.
        metrics.insert(
            "mean_rank_err".into(),
            if scored_groups > 0 {
                sum_mean / scored_groups as f64
            } else {
                0.0
            },
        );
        metrics.insert("max_rank_err".into(), max_err);
        metrics.insert("grid_points".into(), GROUP_GRID_POINTS as f64);
        metrics.insert("items".into(), truth.items as f64);
        // Members, not questions asked; those are `grid_points` times this and
        // are reported as `Comparison::queries`.
        metrics.insert("probes".into(), scored_groups as f64);
        metrics.insert("subpopulations".into(), truth.per_group.len() as f64);
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}
