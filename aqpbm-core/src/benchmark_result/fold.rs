//! Folding a run population into statistics: mean, stddev and 95% CI across the
//! N post-warm-up runs of one measurement. Not data aggregation, and not sketch
//! union/merge — `Operation::Merge` only picks which statistic a merge reports.

use crate::benchmark_result::welford::Welford;

use crate::benchmark_result::schema::{CpuTime, LatencySummary, RunStats};
use crate::metrics::{ItemsPerSec, RunMetrics};

/// The rate this measurement produced: `work / elapsed`.
///
/// One function, not one per operation. A rate is a rate — what it is a rate
/// *of* is the operation's business, and the operation is what picks the field
/// this lands in.
pub fn rate(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        if r.elapsed_ns > 0 {
            w.push(ItemsPerSec::compute(r.work, r.elapsed_ns));
        }
    }
    maybe_runstats(w)
}

/// The same rate, one entry per measured run, so a consumer can draw a box
/// plot without re-running the bench.
pub fn rate_samples(runs: &[RunMetrics]) -> Option<Vec<f64>> {
    let samples: Vec<f64> = runs
        .iter()
        .filter(|r| r.elapsed_ns > 0)
        .map(|r| ItemsPerSec::compute(r.work, r.elapsed_ns))
        .collect();
    (!samples.is_empty()).then_some(samples)
}

/// The timed region, in milliseconds.
pub fn elapsed_ms(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        w.push(r.elapsed_ns as f64 / 1_000_000.0);
    }
    maybe_runstats(w)
}

/// User and system time, when the runs carried it.
pub fn cpu_time_ms(runs: &[RunMetrics]) -> Option<CpuTime> {
    let mut user_w = Welford::new();
    let mut sys_w = Welford::new();
    let mut any = false;
    for r in runs {
        if let (Some(u), Some(s)) = (r.cpu_user_ns, r.cpu_sys_ns) {
            user_w.push(u as f64 / 1_000_000.0);
            sys_w.push(s as f64 / 1_000_000.0);
            any = true;
        }
    }
    any.then(|| CpuTime {
        user_ms: runstats_from(user_w),
        sys_ms: runstats_from(sys_w),
    })
}

/// The memory high-water marks. `max` for every one: each run builds its own
/// sketch, so these are N readings of one quantity and the largest is what a
/// capacity plan has to survive.
pub fn memory_maxima(runs: &[RunMetrics]) -> MemoryMaxima {
    MemoryMaxima {
        rss_peak_kb: runs.iter().filter_map(|r| r.rss_peak_kb).max(),
        heap_allocated_kb: runs.iter().filter_map(|r| r.heap_allocated_kb).max(),
        heap_bytes_net: runs.iter().filter_map(|r| r.heap_bytes_net).max(),
        heap_bytes_peak: runs.iter().filter_map(|r| r.heap_bytes_peak).max(),
    }
}

/// What [`memory_maxima`] returns, so a caller places four numbers by name
/// instead of by tuple position.
pub struct MemoryMaxima {
    pub rss_peak_kb: Option<u64>,
    pub heap_allocated_kb: Option<u64>,
    pub heap_bytes_net: Option<u64>,
    pub heap_bytes_peak: Option<u64>,
}

/// The sketch's self-reported footprint. One quantity, not a population, so
/// the last run's reading stands for all of them.
pub fn memory_bytes(runs: &[RunMetrics]) -> Option<u64> {
    runs.iter().filter_map(|r| r.memory_bytes).next_back()
}

/// The distribution the per-update recorder built, from the last run that has
/// one.
pub fn latency(runs: &[RunMetrics]) -> Option<LatencySummary> {
    runs.iter()
        .rev()
        // An armed recorder that was never fed is not a latency of zero: a body
        // that timed one region instead of per call has no per-call sample, and
        // reporting `p50: 0` there would read as an instantaneous operation.
        .find_map(|r| r.latency_ns.as_ref().filter(|l| l.count > 0))
        .map(|l| LatencySummary {
            p50: l.p50,
            p95: l.p95,
            p99: l.p99,
            p999: l.p999,
            max: l.max,
            count: l.count,
        })
}

/// Fold every run's accuracy scalars into one object. Each repetition drew
/// independently, so spread is real: each key ships as a mean plus `_stddev`,
/// with `accuracy_runs` counting every run — the two can visibly disagree.
pub fn scores(runs: &[RunMetrics]) -> Option<serde_json::Value> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<&str, Welford> = BTreeMap::new();
    let mut n_runs = 0usize;
    for r in runs {
        let Some(m) = r.scores.as_ref() else {
            continue;
        };
        n_runs += 1;
        for (k, v) in m {
            acc.entry(k.as_str()).or_default().push(*v);
        }
    }
    if acc.is_empty() {
        return None;
    }
    let mut out = serde_json::Map::new();
    for (k, w) in acc {
        out.insert(k.to_string(), json_num(w.mean()));
        // A `_stddev` of exactly zero carries no information, and most such
        // keys are configuration constants riding along in the metric map.
        if w.n() > 1 && w.stddev() != 0.0 {
            out.insert(format!("{k}_stddev"), json_num(w.stddev()));
        }
    }
    out.insert("accuracy_runs".into(), serde_json::json!(n_runs));
    Some(serde_json::Value::Object(out))
}

/// `serde_json` refuses non-finite floats; a NaN or inf here means a
/// comparator divided by zero, and emitting `null` says so rather than
/// failing the whole record.
fn json_num(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

fn maybe_runstats(w: Welford) -> Option<RunStats> {
    if w.n() == 0 {
        None
    } else {
        Some(runstats_from(w))
    }
}

/// Summarise the post-warmup iterations of one process. `ci95` is deliberately
/// `None` — these iterations are not independent samples, so no interval over
/// them means what one claims. `--repeats R` fills it in. See `RunStats::ci95`.
fn runstats_from(w: Welford) -> RunStats {
    RunStats {
        mean: w.mean(),
        stddev: w.stddev(),
        ci95: None,
        n: w.n(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rm(work: u64, elapsed_ns: u64) -> RunMetrics {
        RunMetrics {
            work,
            elapsed_ns,
            ..RunMetrics::default()
        }
    }

    /// A rate is `work / elapsed`, and the population is the measured runs.
    #[test]
    fn rate_is_the_mean_of_the_per_run_rates() {
        let runs = vec![rm(1_000_000, 100_000_000), rm(1_000_000, 50_000_000)];
        let r = rate(&runs).expect("two runs");
        // 10M/s and 20M/s.
        assert!((r.mean - 15_000_000.0).abs() < 1.0);
        assert_eq!(r.n, 2);
    }

    /// The samples are the same numbers, unaggregated, so a consumer can draw
    /// the spread without re-running.
    #[test]
    fn samples_cover_the_same_runs_as_the_mean() {
        let runs = vec![rm(1_000_000, 100_000_000), rm(1_000_000, 50_000_000)];
        let s = rate_samples(&runs).expect("two runs");
        assert_eq!(s.len(), 2);
        assert!((s[0] - 10_000_000.0).abs() < 1.0);
        assert!((s[1] - 20_000_000.0).abs() < 1.0);
    }

    /// A run that timed nothing contributes no rate — dividing by zero would
    /// otherwise report an infinite one.
    #[test]
    fn a_run_that_timed_nothing_is_not_a_rate() {
        assert!(rate(&[rm(1000, 0)]).is_none());
        assert!(rate_samples(&[rm(1000, 0)]).is_none());
    }

    /// The timed region in milliseconds, over the same population.
    #[test]
    fn elapsed_is_the_timed_region_not_the_whole_run() {
        let runs = vec![rm(10, 5_000_000), rm(10, 15_000_000)];
        let e = elapsed_ms(&runs).expect("two runs");
        assert!((e.mean - 10.0).abs() < 1e-9);
    }

    /// Scores fold per key across runs, with a stddev only where there is one.
    #[test]
    fn scores_fold_every_key_across_runs() {
        let a = RunMetrics {
            scores: Some([("are_all".to_string(), 0.10)].into_iter().collect()),
            ..Default::default()
        };
        let b = RunMetrics {
            scores: Some([("are_all".to_string(), 0.20)].into_iter().collect()),
            ..Default::default()
        };
        let out = scores(&[a, b]).expect("two runs carried scores");
        let obj = out.as_object().unwrap();
        assert!((obj["are_all"].as_f64().unwrap() - 0.15).abs() < 1e-9);
        assert!(obj.contains_key("are_all_stddev"));
        assert_eq!(obj["accuracy_runs"].as_u64().unwrap(), 2);
    }

    /// A run with no scores is not a score of zero.
    #[test]
    fn no_scores_is_absent_not_zero() {
        assert!(scores(&[RunMetrics::default()]).is_none());
    }
}
