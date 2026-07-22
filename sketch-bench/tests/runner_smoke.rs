//! End-to-end smoke test for `BenchRunner` against a toy
//! counting sketch. Proves: sketch construction → N-run +
//! warmup loop → metrics aggregation → v1 JSONL record.

use aqpbm_core::sketch::{MergeUnsupported, Sketch};
use aqpbm_core::workload::I64Workload;
use sketch_bench::accuracy::cardinality::CardinalityGT;
use sketch_bench::{BenchConfig, BenchRunner, MetricsMask};

/// Trivial exact-counting "sketch" — not a real sketch, but
/// exercises the full trait + runner machinery against a known
/// ground truth.
struct ExactCounter {
    seen: std::collections::HashSet<i64>,
}

impl Sketch for ExactCounter {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.seen.insert(*v);
    }
    fn query(&self, _: ()) -> f64 {
        self.seen.len() as f64
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
    let reports = runner.run(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        Some(&CardinalityGT::default()),
    );

    // `MetricsMask::all()` ⇒ 3 passes (throughput, latency, accuracy).
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
    let reports = BenchRunner::new(cfg, &workload, "exact", "empty").run(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        None::<&CardinalityGT>,
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
    let reports = runner.run(
        || ExactCounter {
            seen: Default::default(),
        },
        |s, it| s.update(it),
        Some(&CardinalityGT::default()),
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
