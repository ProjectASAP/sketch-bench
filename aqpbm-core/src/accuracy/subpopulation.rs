//! Ground truth for the grouped sketches: a statistic taken *within* one
//! subpopulation, one comparator per statistic.
//!
//! A record stream carries `d` label columns and one value, and a grouped
//! sketch stores every column subset. Which population is scored depends on
//! what the cells hold, and the three comparators here differ in exactly that.
//!
//! - [`SubpopFrequencyGT`] scores **(label, value) pairs**: the counters live
//!   inside a group and count values.
//! - [`SubpopCardinalityGT`] scores **subpopulations**: how many distinct
//!   values a group held, which a counter array structurally cannot answer.
//! - [`SubpopRankErrorGT`] scores **subpopulations**, in rank-error units: the
//!   ordered statistic inside a group.
//!
//! The first two ship error in the same vocabulary as the ungrouped frequency
//! comparator (`are_top1`…`are_top1000`, `are_all`, `aae_*`), so a grouped
//! row's numbers land in the columns an ungrouped row already fills. The third
//! ships rank error, matching [`RankErrorGT`](super::quantile::RankErrorGT),
//! because that is the ruler its statistic is defined against.
//!
//! # What these three fix
//!
//! - The population is one label column's groups, at subset depth 1. A sketch
//!   over `d` columns stores `2^d - 1` subsets and is scored on one. See #54.
//! - Every group that occurred is probed, uncapped, in a fixed shuffled order.
//!   Groups that did not occur are never probed, so false positives are not
//!   measured.
//! - Error is the unweighted mean over population members. A member is a
//!   (group, value) pair for [`SubpopFrequencyGT`] and a group for the other
//!   two, which is why `probes` differs between them at one workload.
//!
//! A run wanting different ones writes its own [`GroundTruth`] and its sketch's
//! capability impl, and reuses [`subset_key`], [`rank_and_shuffle`],
//! [`score_counting_population`], [`summarise`] and [`ErrSummary`]. The test at
//! the foot of this file is that swap, done: a depth-2 population, scored
//! without reimplementing any of the above.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use crate::accumulator::Accumulator;
use crate::workload::Labeled;

use super::frequency::percentile;
use super::quantile::{lower_bound, upper_bound, QuantileValue};
use super::statistic::{SubpopCardinalityOps, SubpopFrequencyOps, SubpopQuantileOps};
use super::GroundTruth;

/// Prefix lengths of the true-value ranking at which error is reported.
///
/// Ascending, which [`score_counting_population`] relies on: it stops at the
/// first prefix longer than the population instead of skipping it.
pub const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

pub struct SubpopFrequencyGT {
    /// Which label column the subpopulation is taken over. A grouped sketch
    /// stores every column subset, but each one is its own population with its
    /// own error, so a comparator scores one and names it.
    pub label_column: usize,
}

/// The exact per-(group, value) counts, the ranking, and the unfiltered
/// population. Owned: the truth outlives the borrow of `items`.
pub struct SubpopFreqTruth<V> {
    exact: HashMap<(String, V), u64>,
    ranked: Vec<(String, V)>,
    all: Vec<(String, V)>,
}

impl<S, V> GroundTruth<S> for SubpopFrequencyGT
where
    V: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = Labeled<V>> + SubpopFrequencyOps<Value = V>,
{
    type Truth = SubpopFreqTruth<V>;
    type Probe = (String, V);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopFreqTruth<V> {
        // One pass: a record contributes to exactly one (label, value) pair at
        // this column.
        let mut exact: HashMap<(String, V), u64> = HashMap::new();
        for it in items {
            let Some(group) = subset_key(it, &[self.label_column]) else {
                continue;
            };
            *exact.entry((group, it.value.clone())).or_insert(0) += 1;
        }
        let (ranked, all) = rank_and_shuffle(exact.iter().map(|(k, c)| (k.clone(), *c)));
        SubpopFreqTruth { exact, ranked, all }
    }

    fn probes(&self, truth: &SubpopFreqTruth<V>) -> Vec<(String, V)> {
        // `all` already holds every member exactly once, and the top-k prefixes
        // are prefixes of a permutation of it, so this one sweep answers both.
        truth.all.clone()
    }

    fn ask(&self, sketch: &S, probe: &(String, V)) -> f64 {
        sketch.estimate_subpop_frequency(&[probe.0.as_str()], &probe.1)
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(
        &self,
        truth: &SubpopFreqTruth<V>,
        probes: &[(String, V)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&(String, V), f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        // One member of the population is one (group, value) pair.
        let groups: HashSet<&str> = truth.exact.keys().map(|(g, _)| g.as_str()).collect();
        let mut metrics = score_counting_population(&truth.ranked, &truth.all, groups.len(), |pair| {
            (
                *est.get(pair).unwrap_or(&0.0),
                *truth.exact.get(pair).unwrap_or(&0) as f64,
            )
        });
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}

/// The key a grouped sketch files a label subset under: the named columns joined
/// with `;`, in column order. `None` when the record has no label at one of
/// them.
///
/// This is `Hydra::update`'s internal format, so a comparator that builds its
/// truth any other way is scoring a key space the sketch does not have. That is
/// the mistake worth not making twice, which is why this is here and public
/// rather than inline in each `truth`.
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

/// A population twice over: ranked by `measure` descending with ties on the key
/// ascending, and the same members in probe order.
///
/// The ranking is what `top1`…`top1000` are prefixes of, so it has to be total
/// and seed-free. The probe order is shuffled because the probe loop is timed,
/// and sweeping heaviest-first is not an access pattern a query-throughput
/// number should be read off; the seed is fixed so the order reproduces across
/// runs and implementations. Neither is capped or sampled, because a sampled
/// population is not the truth.
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

// ---------- subpopulation cardinality ----------

/// Ground truth for a grouped cardinality sketch: how many distinct values one
/// subpopulation held.
///
/// The population is the **subpopulation**, one entry per distinct label at the
/// scored column. That is a different population from the one
/// [`SubpopFrequencyGT`] scores, so at the same workload the two report the
/// same `subpopulations` and a different `probes`.
///
/// `top1`…`top1000` rank groups by true distinct count descending, which is the
/// grouped analogue of ranking pairs by frequency: the prefix names the groups
/// a sketch is most likely to be asked about and least able to hide error in.
pub struct SubpopCardinalityGT {
    /// Which label column the subpopulation is taken over.
    pub label_column: usize,
}

/// Exact distinct-value counts per group, plus the ranking and the unfiltered
/// population.
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
        // One pass: a record contributes its value to exactly one group at this
        // column, and the group's truth is the size of that set.
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
        // One member of the population is one group, and `all` holds each once.
        truth.all.clone()
    }

    fn ask(&self, sketch: &S, probe: &String) -> f64 {
        sketch.estimate_subpop_cardinality(&[probe.as_str()])
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(
        &self,
        truth: &SubpopCardTruth,
        probes: &[String],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&String, f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        // One member of the population is one group, so `probes` and
        // `subpopulations` agree here and do not for the frequency comparator.
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

// ---------- subpopulation quantile ----------

/// Number of quantiles probed per group. The same 101-point grid
/// [`RankErrorGT`](super::quantile::RankErrorGT) uses, so a grouped rank error
/// and an ungrouped one are read on the same ruler.
const GROUP_GRID_POINTS: usize = 101;

/// Ground truth for a grouped quantile sketch: the ordered statistic inside one
/// subpopulation, scored in rank-error units.
///
/// Cost is worth stating: this issues `groups * 101` estimate calls, and a
/// grouped sketch answers each one out of several grid rows. It is the most
/// expensive comparator here, and it pays in full: scoring a sample of the
/// groups would report the error of that sample under the name of the whole.
/// A run that is too slow wants fewer groups in the workload.
pub struct SubpopRankErrorGT {
    /// Which label column the subpopulation is taken over.
    pub label_column: usize,
}

/// The ordered statistic inside each group, plus the groups worth probing.
pub struct SubpopRankTruth {
    per_group: HashMap<String, Vec<f64>>,
    /// Every group that occurred, in [`shuffled_keys`] order. Not capped and not
    /// in size order: a three-record group has almost no ranks to be wrong
    /// about and still counts as one member of the population, which is what
    /// makes `mean_rank_err` sensitive to a workload's long tail of tiny
    /// groups.
    probed: Vec<String>,
    /// Records in the whole workload, not in the scored groups. The two differ
    /// only when a record has no label at the scored column, which `truth`
    /// skips.
    items: usize,
}

impl<S, V> GroundTruth<S> for SubpopRankErrorGT
where
    V: QuantileValue,
    S: Accumulator<Item = Labeled<V>> + SubpopQuantileOps,
{
    type Truth = SubpopRankTruth;
    /// One (group, fraction) question.
    type Probe = (String, f64);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopRankTruth {
        // Every value the group carried, sorted. Not deduplicated, because rank
        // is over occurrences.
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
        // Ranked by size, and only the probe order is kept: this comparator
        // reports no top-k prefix, so it has nothing to rank for.
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
                // Same rank-interval rule as the ungrouped comparator: the
                // returned value occupies `[lower, upper]`, so a target inside
                // that interval is not an error.
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
            // The 101 points describe one member of the population, so they are
            // meaned into that member's error before the members are meaned.
            sum_mean += group_sum / GROUP_GRID_POINTS as f64;
            scored_groups += 1;
        }

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        // Mean over members, unweighted, exactly as the two counting
        // comparators do it. A 40000-record group and a 125-record one each
        // count once, so this number moves with a workload's group-size spread
        // and not only with the sketch.
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
        // Members of the population, matching what the other two report under
        // this name. The questions asked are `grid_points` times this, and are
        // reported as `Comparison::queries`.
        metrics.insert("probes".into(), scored_groups as f64);
        metrics.insert("subpopulations".into(), truth.per_group.len() as f64);
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}

// ---------- shared error summary ----------

/// The error summary one probe sweep produces, in the vocabulary the two
/// grouped counting comparators share. Kept as one type so a metric cannot be
/// spelled one way in one comparator and another way in the next.
pub struct ErrSummary {
    pub are: f64,
    pub aae: f64,
    pub l1: f64,
    pub l2: f64,
    pub n: f64,
}

impl ErrSummary {
    /// `are_*` and `aae_*` under `label`, plus `l1_err` / `l2_err` when the
    /// sweep is the unfiltered `all` population. The top-k prefixes have no
    /// norms because a norm over a prefix is not a norm.
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

/// Every metric the two counting comparators report, from the one thing they do
/// not share: `answer`, which gives `(estimate, truth)` for one member of the
/// population.
///
/// `ranked` is the population by true value descending and `all` is the same
/// members in probe order, so `ranked[..k]` is the top-k prefix. Both are
/// borrowed and neither is consumed, which is what lets a prefix re-use the
/// answers already collected instead of re-querying.
///
/// A comparator that wants a different roll-up, weighted by group size say,
/// replaces this call and keeps [`summarise`] and [`ErrSummary`].
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
            // Reporting `are_top1000` over 400 distinct members would mean
            // `are_all` under a name claiming otherwise. Omit it.
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

    // How many distinct groups the scored column carried. A grouped sketch's
    // error is a function of this, so a record without it cannot be read.
    metrics.insert("subpopulations".into(), subpopulations as f64);
    metrics
}

/// Fold `(estimate, truth)` pairs into the summary. Relative error is averaged
/// over the entries with a non-zero truth only, because dividing by a zero
/// truth is undefined and counting it as zero error would reward a miss.
///
/// Every population this module builds comes out of the truth, so every member
/// has a truth of at least 1 and that filter never fires here. It is kept
/// because a population that does probe absent groups, which is what measuring
/// false positives would take, is exactly the case it exists for.
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


#[cfg(test)]
mod tests {
    use super::*;

    /// The estimator that does no work: every subpopulation frequency is zero.
    struct NullSubpop;
    impl Accumulator for NullSubpop {
        type Item = Labeled<i64>;
        fn update(&mut self, _: &Labeled<i64>) {}
    }
    impl SubpopFrequencyOps for NullSubpop {
        type Value = i64;
        fn estimate_subpop_frequency(&self, _: &[&str], _: &i64) -> f64 {
            0.0
        }
    }

    /// The estimator that is exactly right, built from the same pass the
    /// comparator makes. Pins that the comparator's own truth is self-consistent.
    struct ExactSubpop {
        counts: HashMap<(String, i64), u64>,
        column: usize,
    }
    impl Accumulator for ExactSubpop {
        type Item = Labeled<i64>;
        fn update(&mut self, r: &Labeled<i64>) {
            let label = r.label(self.column).unwrap_or("").to_string();
            *self.counts.entry((label, r.value)).or_insert(0) += 1;
        }
    }
    impl SubpopFrequencyOps for ExactSubpop {
        type Value = i64;
        /// Joined, not `labels[0]`: the query names a label *subset*, and the
        /// key a grouped sketch files it under is the join. This is what
        /// `PolarsSubpopFrequency` does, and answering out of the first label
        /// alone is the whole of what a wrapper has to change to serve a deeper
        /// population.
        fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
            *self.counts.get(&(labels.join(";"), *value)).unwrap_or(&0) as f64
        }
    }

    fn records() -> Vec<Labeled<i64>> {
        // key1 ∈ {a, b}, key2 ∈ {x, y}; the value is what gets counted.
        [
            ("a;x", 10),
            ("a;y", 10),
            ("a;x", 20),
            ("b;x", 30),
            ("b;y", 30),
            ("b;y", 30),
        ]
        .iter()
        .map(|(k, v)| Labeled {
            key: (*k).to_string(),
            value: *v,
        })
        .collect()
    }

    /// The property that makes ARE worth reporting: a null estimator scores
    /// exactly 1.0, on every population.
    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let gt = SubpopFrequencyGT {
            label_column: 0,
        };
        let cmp = crate::accuracy::run_probes(&gt, &NullSubpop, &records(), false);
        for key in ["are_all", "are_top1"] {
            let v = cmp.metrics[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        assert!(cmp.metrics["aae_all"] > 0.0);
    }

    /// An exact estimator scores exactly 0.0. Together with the null case this
    /// pins both ends of the scale, so a sketch landing outside them means the
    /// comparator is measuring the wrong thing.
    #[test]
    fn exact_estimator_scores_zero() {
        let items = records();
        let mut exact = ExactSubpop {
            counts: HashMap::new(),
            column: 0,
        };
        for it in &items {
            exact.update(it);
        }
        let gt = SubpopFrequencyGT {
            label_column: 0,
        };
        let cmp = crate::accuracy::run_probes(&gt, &exact, &items, false);
        assert_eq!(cmp.metrics["are_all"], 0.0);
        assert_eq!(cmp.metrics["aae_all"], 0.0);
        assert_eq!(cmp.metrics["l1_err"], 0.0);
    }

    /// The population is (subpopulation, value) pairs, and the grouping really
    /// is by the named column: `b;x` and `b;y` both carry value 30, so grouping
    /// by column 0 pools all three of those records into one pair.
    #[test]
    fn truth_groups_by_the_named_column() {
        let items = records();
        let gt = SubpopFrequencyGT {
            label_column: 0,
        };
        let cmp = crate::accuracy::run_probes(&gt, &NullSubpop, &items, false);
        // Distinct pairs at column 0: (a,10) (a,20) (b,30) → 3 pairs, 2 groups.
        assert_eq!(cmp.metrics["probes_all"], 3.0);
        assert_eq!(cmp.metrics["subpopulations"], 2.0);
        // The heaviest pair is (b, 30) with 3 records, so a null estimator's
        // AAE over the top-1 prefix is exactly that count.
        assert_eq!(cmp.metrics["aae_top1"], 3.0);
        // 3 distinct pairs: the top-10 prefix must be omitted, not aliased.
        assert!(!cmp.metrics.contains_key("are_top10"));
    }

    // ---------- the swap, done ----------
    //
    // What it costs to be unsatisfied with the choices this module fixes. The
    // population below is the depth-2 subset, which is #54's missing case and
    // the one none of the three comparators above can reach.
    //
    // Place one is `DepthTwoFreqGT` and its `GroundTruth` impl. Place two is the
    // sketch's capability impl: no signature moves, because `SubpopFrequencyOps`
    // already takes a label *subset*, and one line inside does, from answering
    // out of `labels[0]` to answering out of the join. The shipped `polars`
    // baseline already had that line; the test double above did not, and this
    // test is what caught it. Everything else is reused: `subset_key` for the key space,
    // `rank_and_shuffle` for the ranking and the probe order, and
    // `score_counting_population` for every metric. Nothing below reimplements
    // ARE, AAE, the norms, the top-k prefixes or the percentile.

    struct DepthTwoFreqGT {
        columns: Vec<usize>,
    }

    impl<S> GroundTruth<S> for DepthTwoFreqGT
    where
        S: Accumulator<Item = Labeled<i64>> + SubpopFrequencyOps<Value = i64>,
    {
        type Truth = SubpopFreqTruth<i64>;
        type Probe = (String, i64);
        type Answer = f64;

        fn truth(&self, items: &[Labeled<i64>]) -> SubpopFreqTruth<i64> {
            let mut exact: HashMap<(String, i64), u64> = HashMap::new();
            for it in items {
                let Some(group) = subset_key(it, &self.columns) else {
                    continue;
                };
                *exact.entry((group, it.value)).or_insert(0) += 1;
            }
            let (ranked, all) = rank_and_shuffle(exact.iter().map(|(k, c)| (k.clone(), *c)));
            SubpopFreqTruth { exact, ranked, all }
        }

        fn probes(&self, truth: &SubpopFreqTruth<i64>) -> Vec<(String, i64)> {
            truth.all.clone()
        }

        fn ask(&self, sketch: &S, probe: &(String, i64)) -> f64 {
            let labels: Vec<&str> = probe.0.split(';').collect();
            sketch.estimate_subpop_frequency(&labels, &probe.1)
        }

        fn answer_as_f64(&self, answer: &f64) -> f64 {
            *answer
        }

        fn score(
            &self,
            truth: &SubpopFreqTruth<i64>,
            probes: &[(String, i64)],
            answers: &[f64],
        ) -> BTreeMap<String, f64> {
            let est: HashMap<&(String, i64), f64> =
                probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
            let groups: HashSet<&str> = truth.exact.keys().map(|(g, _)| g.as_str()).collect();
            score_counting_population(&truth.ranked, &truth.all, groups.len(), |pair| {
                (
                    *est.get(pair).unwrap_or(&0.0),
                    *truth.exact.get(pair).unwrap_or(&0) as f64,
                )
            })
        }
    }

    /// The replacement scores a population the shipped comparators cannot, and
    /// lands on the same scale they do: exactly 1.0 for a null estimator.
    #[test]
    fn a_replacement_reaches_the_depth_two_population() {
        let items = records();
        let gt = DepthTwoFreqGT {
            columns: vec![0, 1],
        };
        let cmp = crate::accuracy::run_probes(&gt, &NullSubpop, &items, false);
        // Depth-2 pairs: (a;x,10) (a;y,10) (a;x,20) (b;x,30) (b;y,30) → 5 pairs
        // over 4 groups, against column 0 alone giving 3 pairs over 2 groups.
        assert_eq!(cmp.metrics["probes_all"], 5.0);
        assert_eq!(cmp.metrics["subpopulations"], 4.0);
        assert_eq!(cmp.metrics["are_all"], 1.0);
        // The heaviest depth-2 pair is (b;y, 30), which occurs twice.
        assert_eq!(cmp.metrics["aae_top1"], 2.0);
        // `label_column` belongs to the depth-1 comparators, so a depth-2
        // population does not claim one.
        assert!(!cmp.metrics.contains_key("label_column"));
    }

    /// And the same replacement scores exactly zero against an exact estimator,
    /// which is what says its truth and its probes agree on the key space.
    #[test]
    fn the_replacement_scores_an_exact_estimator_zero() {
        let items = records();
        let mut exact = ExactSubpop {
            counts: HashMap::new(),
            column: 0,
        };
        // The wrapper needs no change, only the key it files under: this is the
        // `;`-joined subset key the sketch itself would build.
        for it in &items {
            *exact.counts.entry((it.key.clone(), it.value)).or_insert(0) += 1;
        }
        let gt = DepthTwoFreqGT {
            columns: vec![0, 1],
        };
        let cmp = crate::accuracy::run_probes(&gt, &exact, &items, false);
        assert_eq!(cmp.metrics["are_all"], 0.0);
        assert_eq!(cmp.metrics["l1_err"], 0.0);
    }

    /// Scoring a different column is a different population, not a relabelling.
    #[test]
    fn a_different_column_is_a_different_population() {
        let items = records();
        let by_col1 = crate::accuracy::run_probes(
            &SubpopFrequencyGT {
                label_column: 1,
            },
            &NullSubpop,
            &items,
            false,
        );
        // Column 1 pairs: (x,10) (y,10) (x,20) (x,30) (y,30) → 5 pairs, 2 groups.
        assert_eq!(by_col1.metrics["probes_all"], 5.0);
        assert_eq!(by_col1.metrics["subpopulations"], 2.0);
        assert_eq!(by_col1.metrics["label_column"], 1.0);
    }
}
