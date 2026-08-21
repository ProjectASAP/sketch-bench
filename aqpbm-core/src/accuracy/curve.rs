//! The error curve the three counting comparators share: `are_*` / `aae_*` /
//! `probes_*` over the ranking prefixes and the unfiltered population, plus
//! `l1_err` / `l2_err` and the `relative_error_*` aliases. One wire vocabulary.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::hash::Hash;

/// Prefix lengths of the true ranking at which error is reported. Prefixes of
/// one sorted array, so the whole curve costs one sort.
pub(crate) const TOP_K_REPORTED: [usize; 4] = [1, 10, 100, 1000];

/// The ranked keys, shuffled so probe order does not hand the baseline the
/// locality encounter order would; fixed seed, so it is reproducible. No cap: a
/// sampled population is not the truth. Generic over what a key is.
pub(crate) fn shuffled<T: Clone>(ranked: &[(T, u64)]) -> Vec<T> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut out: Vec<T> = ranked.iter().map(|(key, _)| key.clone()).collect();
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
    out
}

/// Everything either population needs, asked once. A prefix re-uses the answers
/// instead of re-querying: an estimate is deterministic given the sketch, so a
/// second call would only measure a warm cache.
/// Deduplicated by an explicit key rather than by the probe itself: the
/// frequency comparators key the float width by its bits, and the grouped ones
/// probe with label vectors that are their own key.
pub(crate) fn union_of<T: Clone, K: Eq + Hash>(
    all: &[T],
    ranked: &[T],
    key: impl Fn(&T) -> K,
) -> Vec<T> {
    let mut seen: HashSet<K> = HashSet::new();
    let mut out: Vec<T> = Vec::with_capacity(all.len());
    for t in all {
        if seen.insert(key(t)) {
            out.push(t.clone());
        }
    }
    let deepest = *TOP_K_REPORTED.last().unwrap_or(&0);
    for t in ranked.iter().take(deepest) {
        if seen.insert(key(t)) {
            out.push(t.clone());
        }
    }
    out
}

pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx]
}

/// The error summary one probe sweep produces. Kept as one type so a metric
/// cannot be spelled one way in one comparator and another way in the next.
struct ErrSummary {
    are: f64,
    aae: f64,
    l1: f64,
    l2: f64,
    n: f64,
}

impl ErrSummary {
    /// `are_*` / `aae_*` / `probes_*` under `label`, plus `l1_err` / `l2_err`
    /// when the sweep is the unfiltered `all` population. The top-k prefixes
    /// have no norms, because a norm over a prefix is not a norm.
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

/// Write the whole curve into `metrics`. `ranked` is the population by true
/// weight descending, `all` the unfiltered one, and `pair` reads
/// `(estimate, truth)`. An empty population writes nothing rather than zeros.
pub(crate) fn error_curve<K>(
    ranked: &[K],
    all: &[K],
    pair: impl Fn(&K) -> (f64, f64),
    metrics: &mut BTreeMap<String, f64>,
) {
    for k in TOP_K_REPORTED {
        if k > ranked.len() {
            break;
        }
        let prefix = &ranked[..k];
        if prefix.is_empty() {
            continue;
        }
        summarise(prefix.iter().map(&pair)).write(&format!("top{k}"), metrics);
    }
    if !all.is_empty() {
        summarise(all.iter().map(&pair)).write("all", metrics);
    }

    // `relative_error_mean` / `probes` name the unfiltered population; `are_all`
    // and `probes_all` are the same numbers under names that say so.
    if let Some(v) = metrics.get("are_all").copied() {
        metrics.insert("relative_error_mean".into(), v);
    }
    if let Some(v) = metrics.get("probes_all").copied() {
        metrics.insert("probes".into(), v);
    }

    let mut errs: Vec<f64> = all
        .iter()
        .filter_map(|k| {
            let (est, truth) = pair(k);
            (truth > 0.0).then(|| (est - truth).abs() / truth)
        })
        .collect();
    errs.sort_by(f64::total_cmp);
    metrics.insert("relative_error_p99".into(), percentile(&errs, 0.99));
}
