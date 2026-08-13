//! Ground truth for the grouped sketches — one statistic per file, as the
//! ungrouped ones have. Shared here: the subset key space, the population
//! ordering, and the error roll-up.

use std::collections::BTreeMap;

use crate::workload::Labeled;

use super::frequency::percentile;

pub mod cardinality;
pub mod frequency;
pub mod quantile;

pub use cardinality::{SubpopCardTruth, SubpopCardinalityGT};
pub use frequency::{SubpopFreqTruth, SubpopFrequencyGT};
pub use quantile::{SubpopRankErrorGT, SubpopRankTruth};

/// Ranking prefixes error is reported at. Ascending: [`score_counting_population`]
/// stops at the first one longer than the population.
pub const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

/// The named columns joined with `;`, in column order — `Hydra::update`'s
/// internal format. A truth built any other way scores a key space the sketch
/// does not have.
pub fn subset_key<V>(item: &Labeled<V>, columns: &[usize]) -> Option<String> {
    let mut out = String::new();
    for (i, c) in columns.iter().enumerate() {
        let part = item.label(*c)?;
        if i > 0 {
            out.push(';');
        }
        out.push_str(part);
    }
    Some(out)
}

/// `(ranked, probe order)`. Ranked by measure descending, ties on the key, so
/// the top-k prefixes are deterministic; probe order shuffled under a fixed seed
/// so the timed sweep is not heaviest-first. Neither is capped.
pub fn rank_and_shuffle<K: Clone + Ord>(members: impl Iterator<Item = (K, u64)>) -> (Vec<K>, Vec<K>) {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut by_measure: Vec<(K, u64)> = members.collect();
    by_measure.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut shuffled: Vec<K> = by_measure.iter().map(|(k, _)| k.clone()).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    shuffled.shuffle(&mut rng);
    (by_measure.into_iter().map(|(k, _)| k).collect(), shuffled)
}

/// are: average relative error
/// aae: average absolute error
/// l1: sum of diff
/// l2: sqrt(sum of diff^2)
pub struct ErrSummary {
    pub are: f64,
    pub aae: f64,
    pub l1: f64,
    pub l2: f64,
    pub n: f64,
}

impl ErrSummary {
    /// Norms only on `all`: a norm over a prefix is not a norm.
    pub fn write(&self, label: &str, metrics: &mut BTreeMap<String, f64>) {
        metrics.insert(format!("are_{label}"), self.are);
        metrics.insert(format!("aae_{label}"), self.aae);
        metrics.insert(format!("probes_{label}"), self.n);
        if label == "all" {
            metrics.insert("l1_err".into(), self.l1);
            metrics.insert("l2_err".into(), self.l2);
        }
    }
}

/// Every metric the two counting ground truths report. `answer` gives
/// `(estimate, truth)` for one member; `ranked[..k]` is the top-k prefix, which
/// re-uses answers already collected rather than re-querying.
pub fn score_counting_population<K>(
    ranked: &[K],
    all: &[K],
    subpopulations: usize,
    answer: impl Fn(&K) -> (f64, f64),
) -> BTreeMap<String, f64> {
    let mut metrics: BTreeMap<String, f64> = BTreeMap::new();

    let population = |members: &[K], label: &str, m: &mut BTreeMap<String, f64>| {
        if members.is_empty() {
            return;
        }
        summarise(members.iter().map(&answer)).write(label, m);
    };

    for k in TOP_K_REPORTED {
        if k > ranked.len() {
            // `are_top1000` over 400 members would be `are_all` under another name.
            break;
        }
        population(&ranked[..k], &format!("top{k}"), &mut metrics);
    }
    population(all, "all", &mut metrics);

    if let Some(v) = metrics.get("are_all").copied() {
        metrics.insert("relative_error_mean".into(), v);
    }
    if let Some(v) = metrics.get("probes_all").copied() {
        metrics.insert("probes".into(), v);
    }
    let mut errs: Vec<f64> = all
        .iter()
        .filter_map(|m| {
            let (est, truth) = answer(m);
            if truth <= 0.0 {
                return None;
            }
            Some((est - truth).abs() / truth)
        })
        .collect();
    errs.sort_by(f64::total_cmp);
    metrics.insert("relative_error_p99".into(), percentile(&errs, 0.99));

    metrics.insert("subpopulations".into(), subpopulations as f64);
    metrics
}

/// ARE is meaned over non-zero truths only: dividing by zero is undefined, and
/// scoring it zero would reward a miss.
pub fn summarise(pairs: impl Iterator<Item = (f64, f64)>) -> ErrSummary {
    let (mut are, mut aae, mut l1, mut l2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut n, mut counted) = (0usize, 0usize);
    for (est, truth) in pairs {
        let diff = (est - truth).abs();
        l1 += diff;
        l2 += diff * diff;
        aae += diff;
        if truth > 0.0 {
            are += diff / truth;
            counted += 1;
        }
        n += 1;
    }
    ErrSummary {
        are: if counted > 0 {
            are / counted as f64
        } else {
            0.0
        },
        aae: if n > 0 { aae / n as f64 } else { 0.0 },
        l1,
        l2: l2.sqrt(),
        n: n as f64,
    }
}
