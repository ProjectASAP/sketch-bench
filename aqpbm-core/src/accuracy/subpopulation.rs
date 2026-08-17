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
//! All three score one label column, named in `label_column`. #54 records that
//! this is a fraction of what the sketch is paying for.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use crate::workload::Labeled;

use super::frequency::percentile;
use super::quantile::{lower_bound, upper_bound, QuantileValue};
use super::GroundTruth;

/// Prefix lengths of the true-frequency ranking at which error is reported.
const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

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

impl<V> GroundTruth<Labeled<V>> for SubpopFrequencyGT
where
    V: Eq + Hash + Ord + Clone,
{
    type Truth = SubpopFreqTruth<V>;
    type Probe = (String, V);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopFreqTruth<V> {
        // One pass: a record contributes to exactly one (label, value) pair at
        // this column.
        let mut exact: HashMap<(String, V), u64> = HashMap::new();
        for it in items {
            let Some(label) = it.label(self.label_column) else {
                continue;
            };
            *exact
                .entry((label.to_string(), it.value.clone()))
                .or_insert(0) += 1;
        }
        // Rank by true count descending, ties on the pair, so every top-k
        // prefix is deterministic across runs and implementations.
        let mut by_count: Vec<((String, V), u64)> =
            exact.iter().map(|(k, c)| (k.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = sample_ranked(&by_count);
        let ranked = by_count.into_iter().map(|(k, _)| k).collect();
        SubpopFreqTruth { exact, ranked, all }
    }

    fn probes(&self, truth: &SubpopFreqTruth<V>) -> Vec<(String, V)> {
        union_of(&truth.all, &truth.ranked)
    }

    fn score(
        &self,
        truth: &SubpopFreqTruth<V>,
        probes: &[(String, V)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&(String, V), f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();

        let population = |pairs: &[(String, V)], label: &str, m: &mut BTreeMap<String, f64>| {
            if pairs.is_empty() {
                return;
            }
            let (mut are, mut aae, mut l1, mut l2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut counted = 0usize;
            for pair in pairs {
                let e = *est.get(pair).unwrap_or(&0.0);
                let t = *truth.exact.get(pair).unwrap_or(&0) as f64;
                let diff = (e - t).abs();
                l1 += diff;
                l2 += diff * diff;
                aae += diff;
                if t > 0.0 {
                    are += diff / t;
                    counted += 1;
                }
            }
            let n = pairs.len() as f64;
            m.insert(
                format!("are_{label}"),
                if counted > 0 {
                    are / counted as f64
                } else {
                    0.0
                },
            );
            m.insert(format!("aae_{label}"), aae / n);
            m.insert(format!("probes_{label}"), n);
            if label == "all" {
                m.insert("l1_err".into(), l1);
                m.insert("l2_err".into(), l2.sqrt());
            }
        };

        for k in TOP_K_REPORTED {
            if k > truth.ranked.len() {
                // Reporting `are_top1000` over 400 distinct pairs would mean
                // `are_all` under a name claiming otherwise. Omit it.
                break;
            }
            population(&truth.ranked[..k], &format!("top{k}"), &mut metrics);
        }
        population(&truth.all, "all", &mut metrics);

        if let Some(v) = metrics.get("are_all").copied() {
            metrics.insert("relative_error_mean".into(), v);
        }
        if let Some(v) = metrics.get("probes_all").copied() {
            metrics.insert("probes".into(), v);
        }
        let mut errs: Vec<f64> = truth
            .all
            .iter()
            .filter_map(|pair| {
                let t = *truth.exact.get(pair).unwrap_or(&0) as f64;
                if t <= 0.0 {
                    return None;
                }
                Some((*est.get(pair).unwrap_or(&0.0) - t).abs() / t)
            })
            .collect();
        errs.sort_by(f64::total_cmp);
        metrics.insert("relative_error_p99".into(), percentile(&errs, 0.99));

        // How many distinct groups the column carried. A grouped sketch's error
        // is a function of this, so a record without it cannot be read.
        let groups: std::collections::BTreeSet<&str> =
            truth.exact.keys().map(|(g, _)| g.as_str()).collect();
        metrics.insert("subpopulations".into(), groups.len() as f64);
        metrics.insert("label_column".into(), self.label_column as f64);
        metrics
    }
}

/// Everything either population needs, asked once. A prefix re-uses the
/// answers instead of re-querying.
fn union_of<T: Clone + Eq + Hash>(all: &[T], ranked: &[T]) -> Vec<T> {
    let mut seen: HashSet<&T> = HashSet::new();
    let mut out: Vec<T> = Vec::with_capacity(all.len());
    for t in all {
        if seen.insert(t) {
            out.push(t.clone());
        }
    }
    let deepest = *TOP_K_REPORTED.last().unwrap_or(&0);
    for t in ranked.iter().take(deepest) {
        if seen.insert(t) {
            out.push(t.clone());
        }
    }
    out
}

/// The ranked keys, shuffled. Shuffled so probe order does not hand the
/// baseline the locality that encounter order would. Fixed seed, so the order
/// is reproducible. No cap: a sampled population is not the truth.
///
/// Generic over what a key is, because the three grouped comparators probe
/// different populations: a (group, value) pair for frequency, a group for
/// cardinality and for the ordered statistic.
fn sample_ranked<T: Clone>(ranked: &[(T, u64)]) -> Vec<T> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut out: Vec<T> = ranked.iter().map(|(key, _)| key.clone()).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
    out
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

impl<V> GroundTruth<Labeled<V>> for SubpopCardinalityGT
where
    V: Eq + Hash,
{
    type Truth = SubpopCardTruth;
    type Probe = String;
    type Answer = f64;

    fn truth(&self, items: &[Labeled<V>]) -> SubpopCardTruth {
        // One pass: a record contributes its value to exactly one group at this
        // column, and the group's truth is the size of that set.
        let mut distinct: HashMap<&str, HashSet<&V>> = HashMap::new();
        for it in items {
            let Some(label) = it.label(self.label_column) else {
                continue;
            };
            distinct.entry(label).or_default().insert(&it.value);
        }
        let exact: HashMap<String, u64> = distinct
            .iter()
            .map(|(group, values)| ((*group).to_string(), values.len() as u64))
            .collect();

        let mut by_count: Vec<(String, u64)> = exact.iter().map(|(g, c)| (g.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = sample_ranked(&by_count);
        let ranked = by_count.into_iter().map(|(g, _)| g).collect();
        SubpopCardTruth { exact, ranked, all }
    }

    fn probes(&self, truth: &SubpopCardTruth) -> Vec<String> {
        union_of(&truth.all, &truth.ranked)
    }

    fn score(
        &self,
        truth: &SubpopCardTruth,
        probes: &[String],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&String, f64> = probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();

        let population = |groups: &[String], label: &str, m: &mut BTreeMap<String, f64>| {
            if groups.is_empty() {
                return;
            }
            let summary = summarise(groups.iter().map(|g| {
                (
                    *est.get(g).unwrap_or(&0.0),
                    *truth.exact.get(g).unwrap_or(&0) as f64,
                )
            }));
            summary.write(label, m);
        };

        for k in TOP_K_REPORTED {
            if k > truth.ranked.len() {
                break;
            }
            population(&truth.ranked[..k], &format!("top{k}"), &mut metrics);
        }
        population(&truth.all, "all", &mut metrics);

        if let Some(v) = metrics.get("are_all").copied() {
            metrics.insert("relative_error_mean".into(), v);
        }
        if let Some(v) = metrics.get("probes_all").copied() {
            metrics.insert("probes".into(), v);
        }
        let mut errs: Vec<f64> = truth
            .all
            .iter()
            .filter_map(|g| {
                let t = *truth.exact.get(g).unwrap_or(&0) as f64;
                if t <= 0.0 {
                    return None;
                }
                Some((*est.get(g).unwrap_or(&0.0) - t).abs() / t)
            })
            .collect();
        errs.sort_by(f64::total_cmp);
        metrics.insert("relative_error_p99".into(), percentile(&errs, 0.99));
        metrics.insert("subpopulations".into(), truth.exact.len() as f64);
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
    /// Groups by size descending, capped: a two-element group has almost no
    /// ranks to be wrong about.
    probed: Vec<String>,
    items: usize,
}

impl<V> GroundTruth<Labeled<V>> for SubpopRankErrorGT
where
    V: QuantileValue,
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
            let Some(label) = it.label(self.label_column) else {
                continue;
            };
            per_group
                .entry(label.to_string())
                .or_default()
                .push(it.value.to_f64());
        }
        for values in per_group.values_mut() {
            values.sort_by(f64::total_cmp);
        }
        let mut by_size: Vec<(String, u64)> = per_group
            .iter()
            .map(|(g, v)| (g.clone(), v.len() as u64))
            .collect();
        by_size.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let probed = sample_ranked(&by_size);
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
            sum_mean += group_sum / GROUP_GRID_POINTS as f64;
            scored_groups += 1;
        }

        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
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
struct ErrSummary {
    are: f64,
    aae: f64,
    l1: f64,
    l2: f64,
    n: f64,
}

impl ErrSummary {
    /// `are_*` and `aae_*` under `label`, plus `l1_err` / `l2_err` when the
    /// sweep is the unfiltered `all` population. The top-k prefixes have no
    /// norms because a norm over a prefix is not a norm.
    fn write(&self, label: &str, metrics: &mut BTreeMap<String, f64>) {
        metrics.insert(format!("are_{label}"), self.are);
        metrics.insert(format!("aae_{label}"), self.aae);
        metrics.insert(format!("probes_{label}"), self.n);
        if label == "all" {
            metrics.insert("l1_err".into(), self.l1);
            metrics.insert("l2_err".into(), self.l2);
        }
    }
}

/// Fold `(estimate, truth)` pairs into the summary. Relative error is averaged
/// over the entries with a non-zero truth only, because dividing by a zero
/// truth is undefined and counting it as zero error would reward a miss.
fn summarise(pairs: impl Iterator<Item = (f64, f64)>) -> ErrSummary {
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
    ///
    /// It has no `update`: the comparator computes its truth from the raw
    /// items, and this double answers 0 whatever it was fed, so nothing needs
    /// to go into it.
    struct NullSubpop;
    impl NullSubpop {
        fn estimate_subpop_frequency(&self, _: &[&str], _: &i64) -> f64 {
            0.0
        }
    }
    fn ask_null(s: &mut NullSubpop, p: &(String, i64)) -> f64 {
        s.estimate_subpop_frequency(&[p.0.as_str()], &p.1)
    }

    /// The estimator that is exactly right, built from the same pass the
    /// comparator makes. Pins that the comparator's own truth is self-consistent.
    struct ExactSubpop {
        counts: HashMap<(String, i64), u64>,
        column: usize,
    }
    impl ExactSubpop {
        fn update(&mut self, r: &Labeled<i64>) {
            let label = r.label(self.column).unwrap_or("").to_string();
            *self.counts.entry((label, r.value)).or_insert(0) += 1;
        }
        fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
            *self
                .counts
                .get(&(labels[0].to_string(), *value))
                .unwrap_or(&0) as f64
        }
    }
    fn ask_exact(s: &mut ExactSubpop, p: &(String, i64)) -> f64 {
        s.estimate_subpop_frequency(&[p.0.as_str()], &p.1)
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
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &ask_null, &mut NullSubpop, &records());
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
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &ask_exact, &mut exact, &items);
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
        let gt = SubpopFrequencyGT { label_column: 0 };
        let cmp = crate::accuracy::run_probes(&gt, &ask_null, &mut NullSubpop, &items);
        // Distinct pairs at column 0: (a,10) (a,20) (b,30) → 3 pairs, 2 groups.
        assert_eq!(cmp.metrics["probes_all"], 3.0);
        assert_eq!(cmp.metrics["subpopulations"], 2.0);
        // The heaviest pair is (b, 30) with 3 records, so a null estimator's
        // AAE over the top-1 prefix is exactly that count.
        assert_eq!(cmp.metrics["aae_top1"], 3.0);
        // 3 distinct pairs: the top-10 prefix must be omitted, not aliased.
        assert!(!cmp.metrics.contains_key("are_top10"));
    }

    /// Scoring a different column is a different population, not a relabelling.
    #[test]
    fn a_different_column_is_a_different_population() {
        let items = records();
        let by_col1 = crate::accuracy::run_probes(
            &SubpopFrequencyGT { label_column: 1 },
            &ask_null,
            &mut NullSubpop,
            &items,
        );
        // Column 1 pairs: (x,10) (y,10) (x,20) (x,30) (y,30) → 5 pairs, 2 groups.
        assert_eq!(by_col1.metrics["probes_all"], 5.0);
        assert_eq!(by_col1.metrics["subpopulations"], 2.0);
        assert_eq!(by_col1.metrics["label_column"], 1.0);
    }
}
