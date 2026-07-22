//! N-run aggregation: mean / stddev / 95% CI over
//! `Vec<RunMetrics>` → one `BenchSection`.

pub mod welford;

use aqpbm_core::report::{BenchSection, CpuTime, LatencySummary, RunStats};

use super::config::MetricsMask;
use super::metrics::{ItemsPerSec, RunMetrics};
use welford::Welford;

/// Roll up a slice of `RunMetrics` into a single `BenchSection`.
///
/// `mask` is the *pass* mask (post-`MetricsMask::passes()` split)
/// that produced these runs. Fields whose bit is not present in
/// `mask` are suppressed (set to `None`) even if the run records
/// happen to carry a value — this is what makes each pass emit
/// only the metric it is responsible for, so downstream consumers
/// can tell "not measured in this pass" apart from "measured and
/// happened to be zero". Throughput / query_throughput depend on
/// the THROUGHPUT and ACCURACY bits respectively; latency on
/// LATENCY; cpu on CPU; rss / heap_allocated on MEMORY. The
/// logical `memory_bytes` (param-derived from the sketch itself)
/// is always emitted because it costs nothing to capture and
/// downstream plots want it on every row.
pub fn aggregate(runs: &[RunMetrics], mask: MetricsMask) -> BenchSection {
    let n = runs.len();

    let (throughput, throughput_samples) = if mask.contains(MetricsMask::THROUGHPUT) {
        let mut w = Welford::new();
        let mut samples: Vec<f64> = Vec::with_capacity(runs.len());
        for r in runs {
            if r.insert_wall_time_ns > 0 {
                let v = ItemsPerSec::compute(r.items_inserted, r.insert_wall_time_ns);
                w.push(v);
                samples.push(v);
            }
        }
        let s = if samples.is_empty() {
            None
        } else {
            Some(samples)
        };
        (maybe_runstats(w), s)
    } else {
        (None, None)
    };

    let query_throughput = if mask.contains(MetricsMask::ACCURACY) {
        let mut w = Welford::new();
        for r in runs {
            if r.query_wall_time_ns > 0 {
                w.push(ItemsPerSec::compute(
                    r.queries_executed,
                    r.query_wall_time_ns,
                ));
            }
        }
        maybe_runstats(w)
    } else {
        None
    };

    // Wall time of the whole iteration is cheap to capture and
    // useful as a sanity check across passes — always emit.
    let wall_time_ms = {
        let mut w = Welford::new();
        for r in runs {
            w.push(r.wall_time_ns as f64 / 1_000_000.0);
        }
        maybe_runstats(w)
    };

    let cpu_time_ms = if mask.contains(MetricsMask::CPU) {
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
        if any {
            Some(CpuTime {
                user_ms: runstats_from(user_w),
                sys_ms: runstats_from(sys_w),
            })
        } else {
            None
        }
    } else {
        None
    };

    let (rss_peak_kb, heap_allocated_kb) = if mask.contains(MetricsMask::MEMORY) {
        (
            runs.iter().filter_map(|r| r.rss_peak_kb).max(),
            runs.iter().filter_map(|r| r.heap_allocated_kb).max(),
        )
    } else {
        (None, None)
    };
    let memory_bytes = runs.iter().filter_map(|r| r.memory_bytes).next_back();

    let latency_ns = if mask.contains(MetricsMask::LATENCY) {
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
    } else {
        None
    };

    let accuracy = if mask.contains(MetricsMask::ACCURACY) {
        merge_accuracy(runs)
    } else {
        None
    };

    let _ = n; // n is implicit in each RunStats.n

    BenchSection {
        throughput_items_per_sec: throughput,
        throughput_samples,
        query_throughput_items_per_sec: query_throughput,
        latency_ns,
        cpu_time_ms,
        wall_time_ms,
        rss_peak_kb,
        heap_allocated_kb,
        memory_bytes,
        accuracy,
        // Filled in by the merge pass, which owns these.
        merge_time_ms: None,
        merge_shards: None,
        merge_supported: None,
    }
}

/// Fold every run's accuracy scalars into one object.
///
/// Each repetition of the accuracy pass measured an **independent draw** of
/// the workload (see `BenchRunner::run_pass`), so these are genuine samples
/// and the spread across them is real. Each key is emitted as its mean, with
/// a `<key>_stddev` companion and one `accuracy_runs` count — scalars stay
/// scalars, so a consumer reading `relative_error_mean` keeps working while
/// gaining the ability to see how much it moved.
///
/// A key present in some runs but not others (a top-k prefix that only some
/// draws had enough distinct keys for) is averaged over the runs that
/// reported it; `accuracy_runs` is the maximum, so a reader can spot the
/// difference.
fn merge_accuracy(runs: &[RunMetrics]) -> Option<serde_json::Value> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<&str, Welford> = BTreeMap::new();
    let mut n_runs = 0usize;
    for r in runs {
        let Some(m) = r.accuracy.as_ref() else {
            continue;
        };
        n_runs += 1;
        for (k, v) in m {
            acc.entry(k.as_str()).or_insert_with(Welford::new).push(*v);
        }
    }
    if acc.is_empty() {
        return None;
    }
    let mut out = serde_json::Map::new();
    for (k, w) in acc {
        out.insert(k.to_string(), json_num(w.mean()));
        // A `_stddev` of exactly zero carries no information and most of
        // these keys are configuration constants (`probes_top10`,
        // `grid_points`, `items`) that only ride along in the metric map.
        // Emitting a companion for each doubled the payload with zeros.
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

/// Summarise the post-warmup iterations of one process.
///
/// `ci95` is deliberately `None`: these iterations are not independent
/// samples of the implementation's throughput, so no interval computed from
/// them would mean what an interval claims. `sketchlib bench --repeats R`
/// fills it in from R separate processes. See `RunStats::ci95`.
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

    fn rm(items: u64, wall_ns: u64, insert_ns: u64) -> RunMetrics {
        RunMetrics {
            items_inserted: items,
            wall_time_ns: wall_ns,
            insert_wall_time_ns: insert_ns,
            ..Default::default()
        }
    }

    #[test]
    fn aggregate_empty_returns_none_metrics() {
        let out = aggregate(&[], MetricsMask::all());
        assert!(out.throughput_items_per_sec.is_none());
    }

    #[test]
    fn aggregate_matches_manual_mean() {
        let runs = vec![
            rm(1_000_000, 100_000_000, 100_000_000), // 10M/s
            rm(1_000_000, 50_000_000, 50_000_000),   // 20M/s
        ];
        let out = aggregate(&runs, MetricsMask::THROUGHPUT);
        let tp = out.throughput_items_per_sec.expect("throughput present");
        assert!((tp.mean - 15_000_000.0).abs() < 1.0);
        assert_eq!(tp.n, 2);
    }

    #[test]
    fn aggregate_suppresses_throughput_when_bit_unset() {
        let runs = vec![rm(1_000_000, 100_000_000, 100_000_000)];
        let out = aggregate(&runs, MetricsMask::LATENCY);
        assert!(out.throughput_items_per_sec.is_none());
    }
}
