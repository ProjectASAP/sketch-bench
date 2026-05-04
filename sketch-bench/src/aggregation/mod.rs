//! N-run aggregation: mean / stddev / 95% CI over
//! `Vec<RunMetrics>` → one `BenchSection`.

pub mod welford;

use sketch_core::report::{BenchSection, CpuTime, LatencySummary, RunStats};

use super::metrics::{ItemsPerSec, RunMetrics};
use welford::Welford;

/// Roll up a slice of `RunMetrics` into a single
/// `BenchSection` ready to ship in a v1 JSONL record.
pub fn aggregate(runs: &[RunMetrics]) -> BenchSection {
    let n = runs.len();

    let throughput = {
        let mut w = Welford::new();
        for r in runs {
            if r.insert_wall_time_ns > 0 {
                w.push(ItemsPerSec::compute(
                    r.items_inserted,
                    r.insert_wall_time_ns,
                ));
            }
        }
        maybe_runstats(w)
    };

    let query_throughput = {
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
    };

    let wall_time_ms = {
        let mut w = Welford::new();
        for r in runs {
            w.push(r.wall_time_ns as f64 / 1_000_000.0);
        }
        maybe_runstats(w)
    };

    let cpu_time_ms = {
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
    };

    let rss_peak_kb = runs.iter().filter_map(|r| r.rss_peak_kb).max();
    let heap_allocated_kb = runs.iter().filter_map(|r| r.heap_allocated_kb).max();
    let memory_bytes = runs.iter().filter_map(|r| r.memory_bytes).next_back();

    let latency_ns = runs
        .iter()
        .rev()
        .find_map(|r| r.latency_ns.as_ref())
        .map(|l| LatencySummary {
            p50: l.p50,
            p95: l.p95,
            p99: l.p99,
            p999: l.p999,
            max: l.max,
            count: l.count,
        });

    // Pick the last run's accuracy — all post-warmup runs share
    // the same workload + ground truth, so any of them is
    // representative. Averaging across accuracy samples needs a
    // family-aware merge that each comparator defines; we do that
    // per-family in `accuracy::` when it matters.
    let accuracy = runs.iter().rev().find_map(|r| r.accuracy.clone());

    let _ = n; // n is implicit in each RunStats.n

    BenchSection {
        throughput_items_per_sec: throughput,
        query_throughput_items_per_sec: query_throughput,
        latency_ns,
        cpu_time_ms,
        wall_time_ms,
        rss_peak_kb,
        heap_allocated_kb,
        memory_bytes,
        accuracy,
    }
}

fn maybe_runstats(w: Welford) -> Option<RunStats> {
    if w.n() == 0 {
        None
    } else {
        Some(runstats_from(w))
    }
}

fn runstats_from(w: Welford) -> RunStats {
    let (lo, hi) = w.ci95();
    RunStats {
        mean: w.mean(),
        stddev: w.stddev(),
        ci95: [lo, hi],
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
        let out = aggregate(&[]);
        assert!(out.throughput_items_per_sec.is_none());
    }

    #[test]
    fn aggregate_matches_manual_mean() {
        let runs = vec![
            rm(1_000_000, 100_000_000, 100_000_000), // 10M/s
            rm(1_000_000, 50_000_000, 50_000_000),   // 20M/s
        ];
        let out = aggregate(&runs);
        let tp = out.throughput_items_per_sec.expect("throughput present");
        assert!((tp.mean - 15_000_000.0).abs() < 1.0);
        assert_eq!(tp.n, 2);
    }
}
