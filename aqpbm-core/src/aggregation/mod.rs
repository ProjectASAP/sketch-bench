//! Folding a run population into statistics. Every function here takes the
//! measured runs and returns one summary, and none of them knows which square
//! was measured: what a record should contain is the caller's question, and
//! the caller already has the answer.

pub mod welford;

use welford::Welford;

use crate::metrics::{ItemsPerSec, RunMetrics};
use crate::report::{BenchSection, CpuTime, LatencySummary, RunStats};

/// Roll up `RunMetrics` into one `BenchSection` for one square of the grid.
/// A field the square did not measure is suppressed, which is what keeps "not
/// measured here" apart from "zero".
///
/// The square decides, not the mask: throughput over insert and throughput
/// over query are the same metric on different operations, and they are not
/// the same column.
/// Ingest rate, `items / insert_wall`, with `prepare` excluded. `None` when no
/// run recorded an insert.
pub fn throughput(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        if r.insert_wall_time_ns > 0 {
            w.push(ItemsPerSec::compute(r.items_inserted, r.insert_wall_time_ns));
        }
    }
    maybe_runstats(w)
}

/// The same rate, one entry per measured run, so a consumer can draw a box
/// plot without re-running the bench.
pub fn throughput_samples(runs: &[RunMetrics]) -> Option<Vec<f64>> {
    let samples: Vec<f64> = runs
        .iter()
        .filter(|r| r.insert_wall_time_ns > 0)
        .map(|r| ItemsPerSec::compute(r.items_inserted, r.insert_wall_time_ns))
        .collect();
    (!samples.is_empty()).then_some(samples)
}

/// The rate a *ready-to-answer* sketch is produced at: `items / (insert_wall +
/// finalize_wall)`. Guarded on the insert, not on the build, so this and
/// [`throughput`] summarise the same runs — their ratio is the whole reason
/// both exist.
pub fn build_throughput(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        if r.insert_wall_time_ns > 0 {
            w.push(ItemsPerSec::compute(r.items_inserted, r.build_wall_time_ns()));
        }
    }
    maybe_runstats(w)
}

/// Wall time of the deferred build, per run.
pub fn finalize_time_ms(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        if r.insert_wall_time_ns > 0 {
            w.push(r.finalize_wall_time_ns as f64 / 1_000_000.0);
        }
    }
    maybe_runstats(w)
}

/// Answers per second, from the counters the probe loop kept.
pub fn query_throughput(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        if r.query_wall_time_ns > 0 {
            w.push(ItemsPerSec::compute(r.queries_executed, r.query_wall_time_ns));
        }
    }
    maybe_runstats(w)
}

/// Wall time of the whole iteration. Cheap to capture and useful as a sanity
/// check across squares.
pub fn wall_time_ms(runs: &[RunMetrics]) -> Option<RunStats> {
    let mut w = Welford::new();
    for r in runs {
        w.push(r.wall_time_ns as f64 / 1_000_000.0);
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
pub fn latency_from_recorder(runs: &[RunMetrics]) -> Option<LatencySummary> {
    runs.iter()
        .rev()
        .find_map(|r| r.latency_ns.as_ref())
        .map(|l| LatencySummary {
            p50: l.p50,
            p95: l.p95,
            p99: l.p99,
            p999: l.p999,
            max: l.max,
            count: l.count,
        })
}

/// The same distribution, built from per-call samples, so a query latency and
/// an insert latency are read on one ruler.
pub fn latency_from_calls(runs: &[RunMetrics]) -> Option<LatencySummary> {
    let mut ns: Vec<u64> = runs
        .iter()
        .filter_map(|r| r.query_calls.as_ref())
        .flat_map(|calls| calls.iter().map(|c| c.nanoseconds))
        .collect();
    if ns.is_empty() {
        return None;
    }
    ns.sort_unstable();
    let at = |q: f64| ns[(((ns.len() - 1) as f64) * q).round() as usize];
    Some(LatencySummary {
        p50: at(0.50),
        p95: at(0.95),
        p99: at(0.99),
        p999: at(0.999),
        max: *ns.last().unwrap(),
        count: ns.len() as u64,
    })
}

/// Fold every run's accuracy scalars into one object. Each repetition drew
/// independently, so spread is real: each key ships as a mean plus `_stddev`,
/// with `accuracy_runs` counting every run — the two can visibly disagree.
pub fn accuracy(runs: &[RunMetrics]) -> Option<serde_json::Value> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<&str, Welford> = BTreeMap::new();
    let mut n_runs = 0usize;
    for r in runs {
        let Some(m) = r.accuracy.as_ref() else {
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

    /// What the insert-throughput square places, assembled the way the runner
    /// does, so these tests read the same shape a record carries.
    struct ThroughputView {
        throughput_items_per_sec: Option<RunStats>,
        throughput_samples: Option<Vec<f64>>,
        build_throughput_items_per_sec: Option<RunStats>,
        finalize_time_ms: Option<RunStats>,
    }
    fn throughput_view(runs: &[RunMetrics]) -> ThroughputView {
        ThroughputView {
            throughput_items_per_sec: throughput(runs),
            throughput_samples: throughput_samples(runs),
            build_throughput_items_per_sec: build_throughput(runs),
            finalize_time_ms: finalize_time_ms(runs),
        }
    }

    fn rm(items: u64, wall_ns: u64, insert_ns: u64) -> RunMetrics {
        RunMetrics {
            items_inserted: items,
            wall_time_ns: wall_ns,
            insert_wall_time_ns: insert_ns,
            ..Default::default()
        }
    }

    fn rm_deferred(items: u64, insert_ns: u64, finalize_ns: u64) -> RunMetrics {
        RunMetrics {
            finalize_wall_time_ns: finalize_ns,
            ..rm(items, insert_ns + finalize_ns, insert_ns)
        }
    }

    /// The property that lets one column serve a mixed panel: where `prepare`
    /// is a no-op the build rate must equal the ingest rate exactly — not
    /// approximately, and not absent.
    #[test]
    fn build_throughput_equals_ingest_when_finalize_is_free() {
        let runs = vec![
            rm(1_000_000, 100_000_000, 100_000_000),
            rm(1_000_000, 50_000_000, 50_000_000),
        ];
        let out = throughput_view(&runs);
        let tp = out.throughput_items_per_sec.expect("ingest present");
        let bt = out.build_throughput_items_per_sec.expect("build present");
        assert_eq!(bt.mean, tp.mean);
        assert_eq!(bt.n, tp.n);
        // Measured and zero, not "not measured": a no-op finalize is a fact
        // about the implementation and the field says so.
        assert_eq!(out.finalize_time_ms.expect("finalize present").mean, 0.0);
    }

    /// A `*/polars` row in miniature: insert is a `Vec::push` and the sketch
    /// is built in finalize. The ingest column is allowed to say 100M/s —
    /// that is what pushing costs — but the build column must not.
    #[test]
    fn deferred_build_cost_lands_in_build_throughput() {
        // 10ms of push + 90ms of engine work over 1M items: 100M/s ingest,
        // 10M/s build.
        let runs = vec![rm_deferred(1_000_000, 10_000_000, 90_000_000)];
        let out = throughput_view(&runs);
        let tp = out.throughput_items_per_sec.expect("ingest present");
        let bt = out.build_throughput_items_per_sec.expect("build present");
        assert!((tp.mean - 100_000_000.0).abs() < 1.0);
        assert!((bt.mean - 10_000_000.0).abs() < 1.0);
        assert!((out.finalize_time_ms.unwrap().mean - 90.0).abs() < 1e-9);
    }

    /// Both columns must summarise the same runs, or their ratio — the whole
    /// reason for reporting two — is taken across different denominators.
    #[test]
    fn both_throughput_columns_cover_the_same_runs() {
        let runs = vec![
            rm_deferred(1_000_000, 10_000_000, 90_000_000),
            // Insert too fast for the clock: dropped from the ingest column,
            // and so from the build column too even though its finalize is
            // perfectly measurable.
            rm_deferred(1_000_000, 0, 90_000_000),
        ];
        let out = throughput_view(&runs);
        assert_eq!(out.throughput_items_per_sec.unwrap().n, 1);
        assert_eq!(out.build_throughput_items_per_sec.unwrap().n, 1);
        assert_eq!(out.finalize_time_ms.unwrap().n, 1);
    }


    #[test]
    fn aggregate_empty_returns_none_metrics() {
        let out = throughput_view(&[]);
        assert!(out.throughput_items_per_sec.is_none());
    }

    #[test]
    fn aggregate_matches_manual_mean() {
        let runs = vec![
            rm(1_000_000, 100_000_000, 100_000_000), // 10M/s
            rm(1_000_000, 50_000_000, 50_000_000),   // 20M/s
        ];
        let out = throughput_view(&runs);
        let tp = out.throughput_items_per_sec.expect("throughput present");
        assert!((tp.mean - 15_000_000.0).abs() < 1.0);
        assert_eq!(tp.n, 2);
    }

}
