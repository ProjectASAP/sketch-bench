//! End-to-end smoke test for `BenchRunner` against a toy
//! counting sketch. Proves: sketch construction → N-run +
//! warmup loop → metrics aggregation → v1 JSONL record.

use aqpbm_core::ops::SketchOps;
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::I64Workload;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::runner::{BenchConfig, BenchRunner, NoGT};
use aqpbm_core::metrics::OperationMask;
use aqpbm_core::metrics::MetricsMask;

/// A row nothing scores is never asked anything, but the type still needs a
/// closure. This is it: never called, because `ground_truth` is `None`.
/// Trivial exact-counting "sketch" — not a real sketch, but
/// exercises the full trait + runner machinery against a known
/// ground truth.
struct ExactCounter {
    seen: std::collections::HashSet<i64>,
}

fn insert_exact(s: &mut ExactCounter, v: &i64) {
    s.seen.insert(*v);
}
/// Set union, exact, so a fold can be checked to cost no accuracy.
fn merge_exact(into: &mut ExactCounter, from: &ExactCounter) {
    into.seen.extend(from.seen.iter().copied());
}
/// The toy sketch's ops, stated the way a real row states them.
const EXACT_OPS: SketchOps<ExactCounter, i64, (), f64> = SketchOps {
    merge: Some(merge_exact),
    prepare: None,
    ask: ask_distinct,
    _item: std::marker::PhantomData,
};
/// The same sketch where nothing scores it — `NoGT`'s probe and answer.
const EXACT_OPS_NOGT: SketchOps<ExactCounter, i64, (), ()> = SketchOps {
    merge: Some(merge_exact),
    prepare: None,
    ask: |_, _| (),
    _item: std::marker::PhantomData,
};

impl MemoryFootprint for ExactCounter {
    fn memory_bytes(&self) -> usize {
        self.seen.capacity() * std::mem::size_of::<i64>()
    }
}

impl ExactCounter {
    fn estimate_distinct(&self) -> f64 {
        self.seen.len() as f64
    }
}

/// How the toy sketch is asked. A free function, not a trait impl: nothing
/// declares "this is a cardinality estimator" any more, so what makes it
/// eligible for `CardinalityGT` is that this closure's shape matches.
fn ask_distinct(s: &mut ExactCounter, _: &()) -> f64 {
    s.estimate_distinct()
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
            insert_exact,
            Some(&CardinalityGT),
            &EXACT_OPS,
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
            insert_exact,
            Some(&CardinalityGT),
            &EXACT_OPS,
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
    // `runs: 3` was asked for, but error is deterministic given (data,
    // parameters) and a workload is drawn once — so looping an accuracy square
    // would report three identical answers as `stddev: 0.0`. It runs once.
    assert_eq!(back.runs, 1);
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

impl DeferredBuilder {
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

/// The deferred row's ops: the work is in `prepare`, so it supplies one.
const DEFERRED_OPS: SketchOps<DeferredBuilder, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(|s| s.prepare()),
    ask: |_, _| (),
    _item: std::marker::PhantomData,
};

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
        operations: OperationMask::INSERT,
        ..Default::default()
    };
    let finalized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let f = finalized.clone();
    let reports = BenchRunner::new(cfg, &workload, "deferred", "slim").run(
        move || DeferredBuilder {
            buf: Vec::new(),
            distinct: 0,
            finalized: f.clone(),
        },
            |s: &mut DeferredBuilder, v: &i64| s.update(v),
            None::<&NoGT>,
            &DEFERRED_OPS,
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
    // And it reaches the JSONL, which is where the number was stranded
    // before: `finalize_wall_time_ns` was measured but had nowhere to go.
    let back: aqpbm_core::Record = serde_json::from_str(&reports[0].to_jsonl()).unwrap();
    assert!(back.bench.unwrap().build_throughput_items_per_sec.is_some());

    // The build's own duration is a different square, and asking for it is how
    // you get it. It rides the same insert loop, so the number is the same one;
    // what changed is that a caller who never asks no longer receives it.
    let prepare = finalize_only(&workload);
    assert!(prepare.finalize_time_ms.as_ref().unwrap().mean > 0.0);
    let back: aqpbm_core::Record = serde_json::from_str(&reports[0].to_jsonl()).unwrap();
    assert!(back.bench.is_some());
}

/// One `(prepare, latency)` request against the deferred builder. Its own call
/// because the cross product of `insert,prepare` with `throughput` would reach
/// `(prepare, throughput)`, which is a hole.
fn finalize_only(workload: &I64Workload) -> aqpbm_core::BenchSection {
    let cfg = BenchConfig {
        runs: 3,
        warmup_runs: 1,
        metrics: MetricsMask::LATENCY,
        operations: OperationMask::PREPARE,
        ..Default::default()
    };
    let finalized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reports = BenchRunner::new(cfg, workload, "deferred", "prepare")
        .run(
            move || DeferredBuilder {
                buf: Vec::new(),
                distinct: 0,
                finalized: finalized.clone(),
            },
            |s: &mut DeferredBuilder, v: &i64| s.update(v),
            None::<&NoGT>,
            &DEFERRED_OPS,
        )
        .expect("every square asked for is measured");
    reports[0].bench.clone()
}

/// The same sketch through the other two paths: whichever `--metrics` flags are
/// passed, build cost must land in the same field, or rows from two invocations
/// are silently incomparable.
#[test]
fn every_path_bills_the_deferred_build_to_the_same_field() {
    let workload = I64Workload::uniform(20_000, 5_000, 11);
    for metrics in [
        MetricsMask::THROUGHPUT,
        // Secondary bits attach to the square and route it through
        // `run_once_clean` instead of the slim path.
        MetricsMask::THROUGHPUT | MetricsMask::MEMORY,
    ] {
        let cfg = BenchConfig {
            runs: 2,
            warmup_runs: 0,
            metrics,
            operations: OperationMask::INSERT,
            ..Default::default()
        };
        let finalized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let f = finalized.clone();
        let reports = BenchRunner::new(cfg, &workload, "deferred", "paths").run(
            move || DeferredBuilder {
                buf: Vec::new(),
                distinct: 0,
                finalized: f.clone(),
            },
            |s: &mut DeferredBuilder, v: &i64| s.update(v),
            None::<&NoGT>,
            &DEFERRED_OPS,
        )
        .expect("every square asked for is measured");
        let bench = &reports[0].bench;
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
    let reports = BenchRunner::new(cfg, &workload, "exact", "empty").run(
        || ExactCounter {
            seen: Default::default(),
        },
            insert_exact,
            None::<&NoGT>,
            &EXACT_OPS_NOGT,
    )
    .expect("an empty request selects no square, so it cannot reach an empty one");
    // Empty mask ⇒ no squares ⇒ no reports.
    assert!(reports.is_empty());
}

/// Scoring a folded sketch means querying it, which is the query operation
/// wearing merge's name. Merge produces no answer, so the square is empty and
/// asking for it is refused rather than answered with somebody else's number.
#[test]
fn merge_has_no_accuracy() {
    let workload = I64Workload::uniform(4_000, 500, 7);
    let cfg = BenchConfig {
        runs: 2,
        warmup_runs: 0,
        metrics: MetricsMask::ACCURACY,
        operations: OperationMask::MERGE,
        merge_shards: 4,
        ..Default::default()
    };
    let err = BenchRunner::new(cfg, &workload, "exact", "smoke")
        .run(
            || ExactCounter {
                seen: Default::default(),
            },
            insert_exact,
            Some(&CardinalityGT),
            &EXACT_OPS,
        )
        .expect_err("merge accuracy is an empty square");
    assert!(
        matches!(
            err,
            aqpbm_core::RunError::NotMeasured {
                operation: "merge",
                metric: "accuracy"
            }
        ),
        "expected the square to be refused by name, got {err:?}"
    );
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
        .run(
            || ExactCounter {
                seen: Default::default(),
            },
            insert_exact,
            None::<&NoGT>,
            &EXACT_OPS_NOGT,
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

/// `metrics::is_measurable` claims to mirror the grid inside `BenchRunner::run`,
/// and a frontend refuses squares by name on the strength of that claim. Nothing
/// in the type system holds the two matches together, so this walks all twelve
/// squares and asserts they agree: whatever `is_measurable` says of a square,
/// running it either produces a report or comes back `NotMeasured`.
///
/// Without this, the two could drift and the frontend would refuse a square the
/// runner measures — or generate a whole workload for one it does not.
#[test]
fn is_measurable_agrees_with_the_runner_on_every_square() {
    use aqpbm_core::cell::RunError;
    use aqpbm_core::metrics::{cells, is_measurable};

    let workload = I64Workload::uniform(200, 50, 7);
    for cell in cells(OperationMask::all(), MetricsMask::all()) {
        let cfg = BenchConfig {
            runs: 1,
            warmup_runs: 0,
            metrics: cell.mask(),
            operations: match cell.operation {
                aqpbm_core::metrics::Operation::Insert => OperationMask::INSERT,
                aqpbm_core::metrics::Operation::Query => OperationMask::QUERY,
                aqpbm_core::metrics::Operation::Merge => OperationMask::MERGE,
                aqpbm_core::metrics::Operation::Prepare => OperationMask::PREPARE,
            },
            // Merge needs something to fold, or it fails for a reason that has
            // nothing to do with whether the square is measurable.
            merge_shards: 2,
            ..Default::default()
        };
        let runner = BenchRunner::new(cfg, &workload, "exact", "smoke");
        let got = runner.run(
            || ExactCounter {
                seen: std::collections::HashSet::new(),
            },
            insert_exact,
            Some(&CardinalityGT),
            &EXACT_OPS,
        );
        let square = (cell.operation.name(), cell.metric.name());
        match got {
            Err(RunError::NotMeasured { .. }) => assert!(
                !is_measurable(cell),
                "{square:?}: runner says nothing measures it, is_measurable disagrees"
            ),
            Ok(_) => assert!(
                is_measurable(cell),
                "{square:?}: runner measured it, is_measurable says it is empty"
            ),
            Err(e) => panic!("{square:?}: failed for an unrelated reason: {e}"),
        }
    }
}
