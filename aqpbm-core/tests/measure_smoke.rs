//! What `measure` promises: it runs a body, times the region the body marks,
//! and reports what the body said it did. Nothing about sketches appears here,
//! which is the point — the same guarantees hold for any closure.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use aqpbm_core::measure::{measure, MeasureConfig, RunOutcome, Timed};
use aqpbm_core::metrics::MetricsMask;

fn cfg(runs: usize, warmup_runs: usize, metrics: MetricsMask) -> MeasureConfig {
    MeasureConfig {
        runs,
        warmup_runs,
        metrics,
    }
}

/// Warm-ups run and are discarded. A body that rebuilds per run pays its setup
/// on them too, which is what they are for.
#[test]
fn warmups_run_but_do_not_appear() {
    let calls = AtomicUsize::new(0);
    let runs = measure(&cfg(3, 2, MetricsMask::THROUGHPUT), |t| {
        calls.fetch_add(1, Ordering::Relaxed);
        t.time(|| std::hint::black_box(0u64));
        RunOutcome {
            work: 7,
            ..Default::default()
        }
    });
    assert_eq!(calls.load(Ordering::Relaxed), 5, "3 measured + 2 warm-up");
    assert_eq!(runs.len(), 3, "only the measured ones are returned");
    assert!(runs.iter().all(|r| r.work == 7));
}

/// Setup outside `time` is not in the number. This is the property the whole
/// design rests on: a query body fills a sketch before it measures anything,
/// and that fill must not land in the query's elapsed time.
#[test]
fn only_the_marked_region_is_timed() {
    let runs = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), |t| {
        // "Setup" — deliberately the expensive part.
        let mut acc = 0u64;
        for i in 0..2_000_000u64 {
            acc = acc.wrapping_add(i);
        }
        std::hint::black_box(acc);
        // The measurement — deliberately trivial.
        t.time(|| std::hint::black_box(1u64));
        RunOutcome {
            work: 1,
            ..Default::default()
        }
    });
    let elapsed = runs[0].elapsed_ns;
    assert!(
        elapsed < 1_000_000,
        "the setup loop leaked into the timed region: {elapsed}ns"
    );
}

/// Several marked regions in one body add up — a body may time more than one
/// stretch and get their sum.
#[test]
fn marked_regions_accumulate() {
    let runs = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), |t| {
        t.time(|| std::thread::sleep(std::time::Duration::from_millis(5)));
        t.time(|| std::thread::sleep(std::time::Duration::from_millis(5)));
        RunOutcome {
            work: 2,
            ..Default::default()
        }
    });
    assert!(runs[0].elapsed_ns >= 9_000_000, "{}", runs[0].elapsed_ns);
}

/// The body reports its own footprint, because a body that owns what it built
/// drops it on the way out and core would have nothing left to measure.
#[test]
fn the_body_reports_its_own_footprint() {
    let runs = measure(&cfg(2, 0, MetricsMask::MEMORY), |t| {
        let v: Vec<u64> = (0..1024).collect();
        t.time(|| std::hint::black_box(v.len()));
        RunOutcome {
            work: v.len() as u64,
            memory_bytes: Some((v.capacity() * 8) as u64),
            ..Default::default()
        }
    });
    assert!(runs.iter().all(|r| r.memory_bytes == Some(8192)));
}

/// `time_each` feeds the latency recorder, and only when the mask asks for it.
#[test]
fn per_call_timing_is_opt_in() {
    let items: Vec<u64> = (0..1000).collect();
    let with = measure(&cfg(1, 0, MetricsMask::LATENCY), |t: &mut Timed| {
        t.time_each(&items, |v| *v + 1);
        RunOutcome {
            work: items.len() as u64,
            ..Default::default()
        }
    });
    assert!(with[0].latency_ns.is_some(), "LATENCY set → a distribution");

    let without = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), |t: &mut Timed| {
        t.time_each(&items, |v| *v + 1);
        RunOutcome {
            work: items.len() as u64,
            ..Default::default()
        }
    });
    assert!(without[0].latency_ns.is_none(), "no LATENCY → no histogram");
}

/// Named scalars pass through untouched; an empty map is absent, not empty.
#[test]
fn scores_pass_through_and_absent_stays_absent() {
    let scored = measure(&cfg(1, 0, MetricsMask::ACCURACY), |t| {
        t.time(|| ());
        let mut scores = BTreeMap::new();
        scores.insert("are_all".to_string(), 0.25);
        RunOutcome {
            work: 1,
            scores,
            ..Default::default()
        }
    });
    assert_eq!(scored[0].scores.as_ref().unwrap()["are_all"], 0.25);

    let plain = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), |t| {
        t.time(|| ());
        RunOutcome {
            work: 1,
            ..Default::default()
        }
    });
    assert!(plain[0].scores.is_none());
}

/// Zero measured runs is legal and yields nothing — the body still runs its
/// warm-ups, which is what a caller asking for zero would expect.
#[test]
fn zero_runs_yields_nothing() {
    let calls = AtomicUsize::new(0);
    let runs = measure(&cfg(0, 2, MetricsMask::THROUGHPUT), |t| {
        calls.fetch_add(1, Ordering::Relaxed);
        t.time(|| ());
        RunOutcome::default()
    });
    assert!(runs.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}
