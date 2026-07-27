//! End-to-end smoke test for `BenchRunner` against a toy
//! counting sketch. Proves: sketch construction → N-run +
//! warmup loop → metrics aggregation → v1 JSONL record.

use aqpbm_core::sketch::{MergeUnsupported, Sketch};
use aqpbm_core::workload::I64Workload;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::CardinalityOps;
use aqpbm_core::runner::{BenchConfig, BenchRunner};
use aqpbm_core::metrics::MetricsMask;

/// Trivial exact-counting "sketch" — not a real sketch, but
/// exercises the full trait + runner machinery against a known
/// ground truth.
struct ExactCounter {
    seen: std::collections::HashSet<i64>,
}

impl Sketch for ExactCounter {
    type Item = i64;
    fn update(&mut self, v: &i64) {
        self.seen.insert(*v);
    }
    fn memory_bytes(&self) -> usize {
        self.seen.capacity() * std::mem::size_of::<i64>()
    }
    /// Set union — exact, so the merge pass can also check that merging
    /// costs no accuracy, which is the property the pass exists to test.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.seen.extend(other.seen.iter().copied());
        Ok(())
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
        metrics: MetricsMask::all(),
        query_count: Some(1),
        ..Default::default()
    };
    let runner = BenchRunner::new(cfg, &workload, "exact", "smoke");
    // The timed passes carry no ground truth; accuracy is scored separately.
    let mut reports = runner.run_timed(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
    );
    reports.extend(runner.run_accuracy(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        &CardinalityGT::default(),
    ));

    // `MetricsMask::all()` ⇒ throughput + latency (timed) + accuracy = 3 passes.
    assert_eq!(reports.len(), 3);
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

    let accuracy = reports
        .iter()
        .find(|r| r.bench.accuracy.is_some())
        .expect("an accuracy pass exists");

    // v1 JSONL record round-trips for each pass.
    let jsonl = accuracy.to_jsonl();
    let back: aqpbm_core::Record = serde_json::from_str(&jsonl).unwrap();
    assert_eq!(back.sketch, "exact");
    assert_eq!(back.impl_name, "smoke");
    assert_eq!(back.runs, 3);
    assert!(back.bench.as_ref().unwrap().accuracy.is_some());
}

/// A sketch shaped like the deferred-build rows (`*/polars`,
/// `lib-fastpath-parallel`): `update` only buffers, and the sketch is built
/// in `finalize_for_query`. Counts its own finalize calls so a test can pin
/// that the runner made them.
struct DeferredBuilder {
    buf: Vec<i64>,
    distinct: usize,
    finalized: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Sketch for DeferredBuilder {
    type Item = i64;
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    fn finalize_for_query(&mut self) {
        self.finalized
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let set: std::collections::HashSet<i64> = self.buf.iter().copied().collect();
        self.distinct = set.len();
        // Enough work that the finalize clock cannot read zero, which is what
        // separates "the runner timed a no-op" from "the runner skipped it".
        std::hint::black_box(&set);
    }
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

/// The bug this pins: a bare `--metrics throughput` takes the slim path in
/// `BenchRunner`, which used to skip `finalize_for_query` outright. For a
/// sketch that defers its build that meant the measured work never ran at
/// all — the `lib-fastpath-parallel` rows reported the cost of buffering a
/// partition they then threw away without inserting it anywhere.
///
/// `MetricsMask::THROUGHPUT` alone, deliberately: adding any other bit routes
/// to `run_once_clean` and stops exercising the path.
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
    let reports = BenchRunner::new(cfg, &workload, "deferred", "slim").run_timed(
        move || DeferredBuilder {
            buf: Vec::new(),
            distinct: 0,
            finalized: f.clone(),
        },
        |s, it| s.update(it),
    );

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

/// The same sketch through the other two paths. Whichever `--metrics` flags
/// are passed, the build cost must land in the same field — the ingest column
/// reporting `Vec::push` on one path and push-plus-build on another would
/// make rows from two invocations silently incomparable.
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
        let reports = BenchRunner::new(cfg, &workload, "deferred", "paths").run_timed(
            move || DeferredBuilder {
                buf: Vec::new(),
                distinct: 0,
                finalized: f.clone(),
            },
            |s, it| s.update(it),
        );
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
        query_count: None,
        ..Default::default()
    };
    let reports = BenchRunner::new(cfg, &workload, "exact", "empty").run_timed(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
    );
    // Empty mask ⇒ no passes ⇒ no reports.
    assert!(reports.is_empty());
}

/// The merge pass publishes post-merge accuracy, so it populates `accuracy`
/// as well as its own fields. A consumer that grouped records by guessing the
/// pass from which fields are set therefore could not tell it from the
/// accuracy pass — `aqpbm-cli::repeat` merged the two and dropped one.
///
/// The record now states its pass, and this pins that it states the *pass*
/// mask and not the aggregation mask: the runner widens the latter with a
/// borrowed ACCURACY bit so the measurement is not suppressed, and reading
/// that back would label this record "accuracy".
#[test]
fn the_merge_pass_is_labelled_merge_not_accuracy() {
    let workload = I64Workload::uniform(4_000, 500, 7);
    let cfg = BenchConfig {
        runs: 2,
        warmup_runs: 0,
        metrics: MetricsMask::MERGE | MetricsMask::ACCURACY,
        merge_shards: 4,
        ..Default::default()
    };
    let runner = BenchRunner::new(cfg, &workload, "exact", "smoke");
    // Both merge and accuracy compare against ground truth, so both belong to
    // the accuracy half of the run.
    let reports = runner.run_accuracy(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        &CardinalityGT::default(),
    );

    let merge = reports
        .iter()
        .find(|r| r.bench.merge_shards.is_some())
        .expect("a merge pass ran");
    assert!(
        merge.bench.accuracy.is_some(),
        "the merge pass should publish post-merge accuracy — otherwise this \
         test is not exercising the collision it exists for"
    );
    assert_eq!(merge.bench.pass.as_deref(), Some("merge"));

    let accuracy = reports
        .iter()
        .find(|r| r.bench.merge_shards.is_none())
        .expect("an accuracy pass ran");
    assert_eq!(accuracy.bench.pass.as_deref(), Some("accuracy"));
}
