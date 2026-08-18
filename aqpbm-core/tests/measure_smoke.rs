//! What `measure` promises: it calls one primed closure per run, times the
//! call, and reports what that closure said it did. Nothing about sketches
//! appears here, which is the point — the same guarantees hold for any closure.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aqpbm_core::measure::{
    measure, record_calls, MeasureConfig, Measurement, Pass, Report, RunOutcome,
};
use aqpbm_core::metrics::MetricsMask;

fn cfg(runs: usize, warmup_runs: usize, metrics: MetricsMask) -> MeasureConfig {
    MeasureConfig {
        runs,
        warmup_runs,
        metrics,
    }
}

/// `n` passes, each running `body` and reporting `work`.
fn passes(n: usize, work: u64, body: impl Fn() + 'static) -> Measurement {
    let body = Rc::new(body);
    (0..n)
        .map(|_| {
            let body = body.clone();
            Box::new(move || {
                body();
                Box::new(move || RunOutcome {
                    work,
                    ..Default::default()
                }) as Report
            }) as Pass
        })
        .collect()
}

/// Warm-ups run and are discarded. Each has a primed closure of its own, so the
/// setup they pay for is the setup a measured run pays — which is what they are
/// for.
#[test]
fn warmups_run_but_do_not_appear() {
    let calls = Rc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let runs = measure(
        &cfg(3, 2, MetricsMask::THROUGHPUT),
        passes(5, 7, move || {
            counted.fetch_add(1, Ordering::Relaxed);
        }),
    );
    assert_eq!(calls.load(Ordering::Relaxed), 5, "3 measured + 2 warm-up");
    assert_eq!(runs.len(), 3, "only the measured ones are returned");
    assert!(runs.iter().all(|r| r.work == 7));
}

/// Setup done while priming the closure is not in the number. This is the
/// property the whole design rests on: a query closure fills a sketch before it
/// is handed over, and that fill must not land in the query's elapsed time.
#[test]
fn only_the_call_is_timed() {
    // "Setup" — deliberately the expensive part, done here rather than inside.
    let mut acc = 0u64;
    for i in 0..2_000_000u64 {
        acc = acc.wrapping_add(i);
    }
    std::hint::black_box(acc);
    let pass: Pass = Box::new(move || {
        std::hint::black_box(1u64);
        Box::new(move || RunOutcome {
            work: 1,
            ..Default::default()
        }) as _
    });

    let runs = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), vec![pass]);
    let elapsed = runs[0].elapsed_ns;
    assert!(
        elapsed < 1_000_000,
        "the setup loop leaked into the timed region: {elapsed}ns"
    );
}

/// The whole call is the region, so everything a pass does is in the number.
#[test]
fn the_whole_call_is_the_region() {
    let runs = measure(
        &cfg(1, 0, MetricsMask::THROUGHPUT),
        passes(1, 2, || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }),
    );
    assert!(runs[0].elapsed_ns >= 9_000_000, "{}", runs[0].elapsed_ns);
}

/// The pass reports its own footprint, after the clock has stopped, because a
/// closure that owns what it built drops it on the way out and core would have
/// nothing left to measure.
#[test]
fn the_pass_reports_its_own_footprint() {
    let measurement: Measurement = (0..2)
        .map(|_| {
            Box::new(move || {
                let v: Vec<u64> = (0..1024).collect();
                std::hint::black_box(v.len());
                Box::new(move || RunOutcome {
                    work: v.len() as u64,
                    memory_bytes: Some((v.capacity() * 8) as u64),
                    ..Default::default()
                }) as Report
            }) as Pass
        })
        .collect();
    let runs = measure(&cfg(2, 0, MetricsMask::MEMORY), measurement);
    assert!(runs.iter().all(|r| r.memory_bytes == Some(8192)));
}

/// A pass that times itself per call reports a distribution; one timed as a
/// single region reports none. Which it is, is the pass's own choice.
#[test]
fn per_call_timing_is_the_passs_choice() {
    let items: Vec<u64> = (0..1000).collect();
    let each: Pass = {
        let items = items.clone();
        Box::new(move || {
            let latency_ns = Some(record_calls(items.len(), |i| {
                std::hint::black_box(items[i] + 1);
            }));
            Box::new(move || RunOutcome {
                work: items.len() as u64,
                latency_ns,
                ..Default::default()
            }) as _
        })
    };
    let with = measure(&cfg(1, 0, MetricsMask::LATENCY), vec![each]);
    assert!(with[0].latency_ns.is_some(), "recorded per call");

    let without = measure(
        &cfg(1, 0, MetricsMask::THROUGHPUT),
        passes(1, items.len() as u64, move || {
            for v in &items {
                std::hint::black_box(*v + 1);
            }
        }),
    );
    assert!(without[0].latency_ns.is_none(), "one region → no histogram");
}

/// Named scalars pass through untouched; an empty map is absent, not empty.
#[test]
fn scores_pass_through_and_absent_stays_absent() {
    let scored: Pass = Box::new(move || {
        Box::new(move || {
            let mut scores = BTreeMap::new();
            scores.insert("are_all".to_string(), 0.25);
            RunOutcome {
                work: 1,
                scores,
                ..Default::default()
            }
        }) as _
    });
    let scored = measure(&cfg(1, 0, MetricsMask::ACCURACY), vec![scored]);
    assert_eq!(scored[0].scores.as_ref().unwrap()["are_all"], 0.25);

    let plain = measure(&cfg(1, 0, MetricsMask::THROUGHPUT), passes(1, 1, || {}));
    assert!(plain[0].scores.is_none());
}

/// Zero measured runs is legal and yields nothing — the warm-up passes still
/// run, which is what a caller asking for zero would expect.
#[test]
fn zero_runs_yields_nothing() {
    let calls = Rc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let runs = measure(
        &cfg(0, 2, MetricsMask::THROUGHPUT),
        passes(2, 0, move || {
            counted.fetch_add(1, Ordering::Relaxed);
        }),
    );
    assert!(runs.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}
