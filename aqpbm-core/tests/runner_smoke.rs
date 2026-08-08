//! End-to-end smoke test for `BenchRunner` against a toy
//! counting sketch. Proves: sketch construction → N-run +
//! warmup loop → metrics aggregation → v1 JSONL record.

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::I64Workload;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::CardinalityOps;
use aqpbm_core::runner::{BenchConfig, BenchRunner, NoGT};
use aqpbm_core::metrics::OperationMask;
use aqpbm_core::metrics::MetricsMask;

/// Trivial exact-counting "sketch" — not a real sketch, but
/// exercises the full trait + runner machinery against a known
/// ground truth.
struct ExactCounter {
    seen: std::collections::HashSet<i64>,
}

impl Accumulator for ExactCounter {
    type Item = i64;
    fn update(&mut self, v: &i64) {
        self.seen.insert(*v);
    }
    /// Set union — exact, so the merge pass can also check that merging
    /// costs no accuracy, which is the property the pass exists to test.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.seen.extend(other.seen.iter().copied());
        Ok(())
    }
}

impl MemoryFootprint for ExactCounter {
    fn memory_bytes(&self) -> usize {
        self.seen.capacity() * std::mem::size_of::<i64>()
    }
}

/// Declares the toy sketch a cardinality estimator, which is what makes it
/// eligible for `CardinalityGT` below.
impl CardinalityOps for ExactCounter {
    fn estimate_distinct(&self) -> f64 {
        self.seen.len() as f64
    }
}

#[test]
fn runner_end_to_end_produces_valid_jsonl() {
    let workload = I64Workload::uniform(10_000, 5_000, 42);
    let cfg = BenchConfig {
        runs: 3,
        warmup_runs: 1,
        // Every square this asks for is one something measures. A request
        // reaching an empty square is an error, which the grid test covers.
        metrics: MetricsMask::THROUGHPUT | MetricsMask::LATENCY,
        operations: OperationMask::INSERT,
        ..Default::default()
    };
    let runner = BenchRunner::new(cfg, &workload, "exact", "smoke");
    let reports = runner
        .run(
            || ExactCounter {
                seen: Default::default(),
            },
            |s, it| s.update(it),
            Some(&CardinalityGT::default()),
        )
        .expect("every square asked for is measured");

    assert_eq!(reports.len(), 2);
    for r in &reports {
        assert_eq!(r.per_run.len(), 3);
    }

    let throughput = reports
        .iter()
        .find(|r| r.bench.throughput_items_per_sec.is_some())
        .expect("a throughput pass exists");
    let tp = throughput.bench.throughput_items_per_sec.as_ref().unwrap();
    assert!(tp.mean > 0.0);
    assert_eq!(tp.n, 3);

    // The query side, asked for on its own.
    let cfg = BenchConfig {
        runs: 3,
        warmup_runs: 1,
        metrics: MetricsMask::ACCURACY,
        operations: OperationMask::QUERY,
        ..Default::default()
    };
    let scored = BenchRunner::new(cfg, &workload, "exact", "smoke")
        .run(
            || ExactCounter {
                seen: Default::default(),
            },
            |s, it| s.update(it),
            Some(&CardinalityGT::default()),
        )
        .expect("query accuracy is measured");
    let accuracy = scored
        .iter()
        .find(|r| r.bench.accuracy.is_some())
        .expect("an accuracy record exists");

    // v1 JSONL record round-trips.
    let jsonl = accuracy.to_jsonl();
    let back: aqpbm_core::Record = serde_json::from_str(&jsonl).unwrap();
    assert_eq!(back.sketch, "exact");
    assert_eq!(back.impl_name, "smoke");
    assert_eq!(back.runs, 3);
    assert!(back.bench.as_ref().unwrap().accuracy.is_some());
}

/// A sketch shaped like the deferred-build rows: `update` only buffers, and the
/// sketch is built in `prepare`. Counts its own finalize calls so a test can
/// pin that the runner made them.
struct DeferredBuilder {
    buf: Vec<i64>,
    distinct: usize,
    finalized: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Accumulator for DeferredBuilder {
    type Item = i64;
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    fn prepare(&mut self) {
        self.finalized
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let set: std::collections::HashSet<i64> = self.buf.iter().copied().collect();
        self.distinct = set.len();
        // Enough work that the finalize clock cannot read zero, which is what
        // separates "the runner timed a no-op" from "the runner skipped it".
        std::hint::black_box(&set);
    }
}

impl MemoryFootprint for DeferredBuilder {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

/// Pins that the slim path taken by a bare `--metrics throughput` still runs
/// `prepare` — without it a deferred-build sketch never does its measured work.
/// THROUGHPUT alone: any other bit routes elsewhere and stops testing this.
#[test]
fn the_slim_throughput_path_still_builds_the_sketch() {
    use std::sync::atomic::Ordering;

    let workload = I64Workload::uniform(20_000, 5_000, 11);
    let cfg = BenchConfig {
        runs: 3,
        warmup_runs: 1,
        metrics: MetricsMask::THROUGHPUT,
        ..Default::default()
    };
    let finalized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let f = finalized.clone();
    let reports = BenchRunner::new(cfg, &workload, "deferred", "slim").run::<_, _, NoGT, _>(
        move || DeferredBuilder {
            buf: Vec::new(),
            distinct: 0,
            finalized: f.clone(),
        },
        |s, it| s.update(it),
        None,
    )
    .expect("every square asked for is measured");

    // Once per trial, warm-ups included — a warm-up that skipped the build
    // would leave the first measured run paying cold-path costs.
    assert_eq!(finalized.load(Ordering::Relaxed), 4);

    let bench = &reports[0].bench;
    let ingest = bench.throughput_items_per_sec.as_ref().unwrap();
    let build = bench
        .build_throughput_items_per_sec
        .as_ref()
        .expect("the slim path reports a build rate too");
    assert_eq!(build.n, 3);
    assert!(
        build.mean < ingest.mean,
        "a deferred build must cost something: build {} !< ingest {}",
        build.mean,
        ingest.mean
    );
    assert!(bench.finalize_time_ms.as_ref().unwrap().mean > 0.0);

    // And it reaches the JSONL, which is where the number was stranded
    // before: `finalize_wall_time_ns` was measured but had nowhere to go.
    let back: aqpbm_core::Record = serde_json::from_str(&reports[0].to_jsonl()).unwrap();
    let bench = back.bench.unwrap();
    assert!(bench.build_throughput_items_per_sec.is_some());
    assert!(bench.finalize_time_ms.is_some());
}

/// The same sketch through the other two paths: whichever `--metrics` flags are
/// passed, build cost must land in the same field, or rows from two invocations
/// are silently incomparable.
#[test]
fn every_path_bills_the_deferred_build_to_the_same_field() {
    let workload = I64Workload::uniform(20_000, 5_000, 11);
    for metrics in [
        MetricsMask::THROUGHPUT,
        // Secondary bits attach to the primary pass and route it through
        // `run_once_clean` instead of the slim path.
        MetricsMask::THROUGHPUT | MetricsMask::MEMORY,
    ] {
        let cfg = BenchConfig {
            runs: 2,
            warmup_runs: 0,
            metrics,
            ..Default::default()
        };
        let finalized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let f = finalized.clone();
        let reports = BenchRunner::new(cfg, &workload, "deferred", "paths").run::<_, _, NoGT, _>(
            move || DeferredBuilder {
                buf: Vec::new(),
                distinct: 0,
                finalized: f.clone(),
            },
            |s, it| s.update(it),
            None,
        )
        .expect("every square asked for is measured");
        let bench = &reports[0].bench;
        assert!(
            bench.finalize_time_ms.as_ref().unwrap().mean > 0.0,
            "{metrics:?} lost the finalize timing"
        );
        assert!(
            bench.build_throughput_items_per_sec.as_ref().unwrap().mean
                < bench.throughput_items_per_sec.as_ref().unwrap().mean,
            "{metrics:?} billed the build to the ingest column"
        );
    }
}

#[test]
fn runner_respects_mask_noop_when_empty() {
    let workload = I64Workload::uniform(1_000, 100, 1);
    let cfg = BenchConfig {
        runs: 2,
        warmup_runs: 0,
        metrics: MetricsMask::empty(),
        ..Default::default()
    };
    let reports = BenchRunner::new(cfg, &workload, "exact", "empty").run::<_, _, NoGT, _>(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        None,
    )
    .expect("an empty request selects no square, so it cannot reach an empty one");
    // Empty mask ⇒ no squares ⇒ no reports.
    assert!(reports.is_empty());
}

/// Merge is an operation and accuracy is a metric, so the two measurements of
/// the merge operation are told apart by the pair. Pins the property a
/// consumer grouping records depends on: no two of them share it.
#[test]
fn the_two_merge_measurements_carry_different_names() {
    let workload = I64Workload::uniform(4_000, 500, 7);
    let cfg = BenchConfig {
        runs: 2,
        warmup_runs: 0,
        metrics: MetricsMask::ACCURACY,
        operations: OperationMask::QUERY | OperationMask::MERGE,
        merge_shards: 4,
        ..Default::default()
    };
    let runner = BenchRunner::new(cfg, &workload, "exact", "smoke");
    // Both cells compare against ground truth, so both belong to the accuracy
    // half of the run.
    let reports = runner.run(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        Some(&CardinalityGT::default()),
    )
    .expect("every square asked for is measured");

    let merge = reports
        .iter()
        .find(|r| r.bench.merge_shards.is_some())
        .expect("a merge measurement ran");
    assert!(
        merge.bench.accuracy.is_some(),
        "merge accuracy is the point of this cell — otherwise this test is \
         not exercising the collision it exists for"
    );
    assert_eq!(merge.bench.operation.as_deref(), Some("merge"));
    assert_eq!(merge.bench.metric.as_deref(), Some("accuracy"));

    let query = reports
        .iter()
        .find(|r| r.bench.merge_shards.is_none())
        .expect("a query measurement ran");
    assert_eq!(query.bench.operation.as_deref(), Some("query"));
    assert_eq!(query.bench.metric.as_deref(), Some("accuracy"));

    // The pair is what separates them; the metric alone no longer does.
    assert_ne!(merge.bench.operation, query.bench.operation);
}

/// A square places its own metric and nothing else. The folding functions do
/// not know which square is being folded, so this is the property that keeps a
/// record from carrying a column it did not measure.
#[test]
fn a_square_places_only_its_own_metric() {
    let workload = I64Workload::uniform(4_000, 500, 7);
    let cfg = BenchConfig {
        runs: 2,
        warmup_runs: 0,
        metrics: MetricsMask::THROUGHPUT | MetricsMask::LATENCY,
        operations: OperationMask::INSERT,
        ..Default::default()
    };
    let reports = BenchRunner::new(cfg, &workload, "exact", "squares")
        .run::<_, _, NoGT, _>(
            || ExactCounter {
                seen: Default::default(),
            },
            |s, it| s.update(it),
            None,
        )
        .expect("both squares are measured");

    let throughput = reports
        .iter()
        .find(|r| r.bench.metric.as_deref() == Some("throughput"))
        .expect("the throughput square ran");
    assert!(throughput.bench.throughput_items_per_sec.is_some());
    assert!(throughput.bench.latency_ns.is_none());

    let latency = reports
        .iter()
        .find(|r| r.bench.metric.as_deref() == Some("latency"))
        .expect("the latency square ran");
    assert!(latency.bench.latency_ns.is_some());
    assert!(latency.bench.throughput_items_per_sec.is_none());
    // The deferred build's columns ride on the insert throughput square, so
    // they must not appear on the latency one either.
    assert!(latency.bench.build_throughput_items_per_sec.is_none());
    assert!(latency.bench.finalize_time_ms.is_none());
}
