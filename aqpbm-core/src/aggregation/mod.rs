//! N-run aggregation: mean / stddev / 95% CI over
//! `Vec<RunMetrics>` → one `BenchSection`.

pub mod welford;

use welford::Welford;

use crate::metrics::{Cell, ItemsPerSec, Metric, MetricsMask, Operation, RunMetrics};
use crate::report::{BenchSection, CpuTime, LatencySummary, RunStats};

/// Roll up `RunMetrics` into one `BenchSection` for one square of the grid.
/// A field the square did not measure is suppressed, which is what keeps "not
/// measured here" apart from "zero".
///
/// The square decides, not the mask: throughput over insert and throughput
/// over query are the same metric on different operations, and they are not
/// the same column.
pub fn aggregate(runs: &[RunMetrics], cell: Cell) -> BenchSection {
    let n = runs.len();
    let mask = cell.mask();

    let (throughput, throughput_samples, build_throughput, finalize_time_ms) =
        if (cell.operation, cell.metric) == (Operation::Insert, Metric::Throughput) {
            let mut w = Welford::new();
            let mut build_w = Welford::new();
            let mut fin_w = Welford::new();
            let mut samples: Vec<f64> = Vec::with_capacity(runs.len());
            for r in runs {
                if r.insert_wall_time_ns > 0 {
                    let v = ItemsPerSec::compute(r.items_inserted, r.insert_wall_time_ns);
                    w.push(v);
                    samples.push(v);
                    // Same guard, not `build_ns > 0`: the two columns must
                    // summarise the same runs, or their ratio — the whole
                    // reason both exist — spans different denominators.
                    build_w.push(ItemsPerSec::compute(
                        r.items_inserted,
                        r.build_wall_time_ns(),
                    ));
                    fin_w.push(r.finalize_wall_time_ns as f64 / 1_000_000.0);
                }
            }
            let s = if samples.is_empty() {
                None
            } else {
                Some(samples)
            };
            (
                maybe_runstats(w),
                s,
                maybe_runstats(build_w),
                maybe_runstats(fin_w),
            )
        } else {
            (None, None, None, None)
        };

    // Issuing queries is what the comparator does, so both squares that
    // measure the query operation get their number from the same two counters.
    let query_throughput = if cell.operation == Operation::Query
        || (cell.operation == Operation::Merge && cell.metric == Metric::Accuracy)
    {
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
    // The tracking allocator's readings, on the same bit as the other memory
    // fields. `max` for both: every run builds its own sketch, so these are N
    // readings of one quantity, and the largest is the one a capacity plan has
    // to survive. Absent unless the linking binary installed the shim.
    let (heap_bytes_net, heap_bytes_peak) = if mask.contains(MetricsMask::MEMORY) {
        (
            runs.iter().filter_map(|r| r.heap_bytes_net).max(),
            runs.iter().filter_map(|r| r.heap_bytes_peak).max(),
        )
    } else {
        (None, None)
    };
    let memory_bytes = runs.iter().filter_map(|r| r.memory_bytes).next_back();

    let latency_ns = match (cell.operation, cell.metric) {
        // Insert latency comes off the `Probe` boundary the recorder wears.
        (Operation::Insert, Metric::Latency) => runs
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
            }),
        // Query latency comes off the per-call samples the probe loop took.
        (Operation::Query, Metric::Latency) => {
            let mut ns: Vec<u64> = runs
                .iter()
                .filter_map(|r| r.query_calls.as_ref())
                .flat_map(|calls| calls.iter().map(|c| c.nanoseconds))
                .collect();
            summarise_latency(&mut ns)
        }
        _ => None,
    };

    let accuracy = if cell.metric == Metric::Accuracy {
        merge_accuracy(runs)
    } else {
        None
    };

    let _ = n; // n is implicit in each RunStats.n

    BenchSection {
        // Folding numbers is this function's whole job. Which measurement they
        // belong to is decided where the measurement is dispatched, and the
        // runner stamps it on the way out.
        metric: None,
        operation: None,
        throughput_items_per_sec: throughput,
        throughput_samples,
        build_throughput_items_per_sec: build_throughput,
        finalize_time_ms,
        query_throughput_items_per_sec: query_throughput,
        latency_ns,
        cpu_time_ms,
        wall_time_ms,
        rss_peak_kb,
        heap_allocated_kb,
        memory_bytes,
        heap_bytes_net,
        heap_bytes_peak,
        accuracy,
        // Filled in by the merge pass, which owns these.
        merge_time_ms: None,
        merge_shards: None,
        merge_supported: None,
    }
}

/// Fold every run's accuracy scalars into one object. Each repetition drew
/// independently, so spread is real: each key ships as a mean plus `_stddev`,
/// with `accuracy_runs` counting every run — the two can visibly disagree.
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

    /// The two squares these tests fold for.
    fn insert(metric: Metric) -> Cell {
        Cell {
            operation: Operation::Insert,
            metric,
            secondary: MetricsMask::empty(),
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
        let out = aggregate(&runs, insert(Metric::Throughput));
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
        let out = aggregate(&runs, insert(Metric::Throughput));
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
        let out = aggregate(&runs, insert(Metric::Throughput));
        assert_eq!(out.throughput_items_per_sec.unwrap().n, 1);
        assert_eq!(out.build_throughput_items_per_sec.unwrap().n, 1);
        assert_eq!(out.finalize_time_ms.unwrap().n, 1);
    }

    #[test]
    fn build_throughput_is_suppressed_with_the_throughput_bit() {
        let runs = vec![rm_deferred(1_000_000, 10_000_000, 90_000_000)];
        let out = aggregate(&runs, insert(Metric::Latency));
        assert!(out.build_throughput_items_per_sec.is_none());
        assert!(out.finalize_time_ms.is_none());
    }

    #[test]
    fn aggregate_empty_returns_none_metrics() {
        let out = aggregate(&[], insert(Metric::Throughput));
        assert!(out.throughput_items_per_sec.is_none());
    }

    #[test]
    fn aggregate_matches_manual_mean() {
        let runs = vec![
            rm(1_000_000, 100_000_000, 100_000_000), // 10M/s
            rm(1_000_000, 50_000_000, 50_000_000),   // 20M/s
        ];
        let out = aggregate(&runs, insert(Metric::Throughput));
        let tp = out.throughput_items_per_sec.expect("throughput present");
        assert!((tp.mean - 15_000_000.0).abs() < 1.0);
        assert_eq!(tp.n, 2);
    }

    #[test]
    fn aggregate_suppresses_throughput_when_bit_unset() {
        let runs = vec![rm(1_000_000, 100_000_000, 100_000_000)];
        let out = aggregate(&runs, insert(Metric::Latency));
        assert!(out.throughput_items_per_sec.is_none());
    }
}

/// Fold per-call durations into the same summary shape the insert recorder
/// produces, so a query latency and an insert latency are read on one ruler.
fn summarise_latency(ns: &mut Vec<u64>) -> Option<LatencySummary> {
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
