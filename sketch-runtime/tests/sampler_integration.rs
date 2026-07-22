//! Integration tests for the three-level enable/disable matrix.
//!
//! * `enabled` feature on (default): the full sampler runs, emits
//!   records, and respects count/time windows.
//! * `Sampler::disabled(...)`: no records emitted, no hot-path
//!   work.
//! * `RuntimeSwitch::disable()`: live flip to no records.
//!
//! The compile-time path (`default-features = false`) is covered
//! by a separate command-line invocation documented in the
//! crate-level README — a single source of truth keeps the same
//! downstream code compiling either way.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aqpbm_core::probe::Probe;
use aqpbm_core::report::{Record, Source};
use aqpbm_core::sketch::Sketch;
use sketch_runtime::exporter::Exporter;
use sketch_runtime::sampler::{Sampler, Tag};
use sketch_runtime::switch::RuntimeSwitch;

#[derive(Default)]
struct RecordSink {
    records: Arc<Mutex<Vec<Record>>>,
}
impl RecordSink {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn count(&self) -> usize {
        self.records.lock().unwrap().len()
    }
    fn clone_shared(&self) -> Self {
        Self {
            records: self.records.clone(),
        }
    }
}
impl Exporter for RecordSink {
    fn export(&self, record: &Record) {
        self.records.lock().unwrap().push(record.clone());
    }
}

struct DummySketch {
    n: u64,
}
impl Sketch for DummySketch {
    type Item = i64;
    type Query = ();
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.n = self.n.wrapping_add(*v as u64);
    }
    fn query(&self, _: ()) -> u64 {
        self.n
    }
    fn memory_bytes(&self) -> usize {
        8
    }
}

fn tag() -> Tag {
    Tag::new("dummy", "test", Source::DataCollector)
}

#[test]
fn every_n_emits_on_window_close() {
    let sink = RecordSink::new();
    let sampler = Sampler::every_n(4, 2, sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);

    // With sample_every_n=4 and samples_per_window=2, we need
    // 4 * 2 = 8 ops to trigger one window emit.
    for i in 0..8 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 1, "one record per 8 ops");
}

#[test]
fn disabled_sampler_emits_nothing() {
    let sink = RecordSink::new();
    let sampler = Sampler::disabled(sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
    for i in 0..10_000 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 0);
}

#[test]
fn runtime_switch_disables_live() {
    let sink = RecordSink::new();
    let switch = RuntimeSwitch::on();
    let sampler = Sampler::every_n(2, 2, sink.clone_shared(), tag()).with_switch(switch.clone());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);

    // 2/2 configuration — 4 ops should produce one record.
    for i in 0..4 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 1);

    // Flip the switch off — subsequent ops are hot-path no-ops,
    // zero further records.
    switch.disable();
    for i in 0..10_000 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 1, "switch disabled → no further records");

    // Flip back on, push another window's worth — one more record.
    switch.enable();
    for i in 0..4 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 2);
}

#[test]
fn time_window_emits_on_time_boundary() {
    let sink = RecordSink::new();
    let sampler =
        Sampler::every_n_time_window(1, Duration::from_millis(50), sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
    // Feed ops; sleep past the window; next op triggers emit.
    probe.update(&0);
    std::thread::sleep(Duration::from_millis(60));
    probe.update(&1);
    assert!(sink.count() >= 1, "time window should have fired");
}

#[test]
fn every_period_samples_at_most_once_per_period() {
    // 20 ms sampling period, 3 samples per window → one record
    // per ~60 ms of wall time, regardless of how many ops we push.
    let sink = RecordSink::new();
    let sampler = Sampler::every_period(Duration::from_millis(20), 3, sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);

    // Burst 10 k ops — at most one should be sampled (the first,
    // which seeds the clock).
    for i in 0..10_000 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 0, "burst is rate-limited to 1 sample");

    // Sleep past 2 × period; burst another 10 k — at most 1 more
    // sample (elapsed > period at the start of the burst).
    std::thread::sleep(Duration::from_millis(25));
    for i in 0..10_000 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 0, "still short of 3 samples");

    // Third wall-period → third sample → window close, one record.
    std::thread::sleep(Duration::from_millis(25));
    probe.update(&1);
    assert_eq!(sink.count(), 1, "one record per 3 samples × 20 ms period");
}

#[test]
fn every_n_time_window_batches_emits() {
    // The "scrape every op, emit once per second" pattern —
    // accumulate every op's latency in memory, emit one batched
    // record on the time boundary.
    let sink = RecordSink::new();
    let sampler =
        Sampler::every_n_time_window(1, Duration::from_millis(30), sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);

    // Feed a handful of ops fast — no record yet (under the
    // 30 ms window).
    for i in 0..1000 {
        probe.update(&i);
    }
    assert_eq!(sink.count(), 0);

    // Wait past the window; next op flushes.
    std::thread::sleep(Duration::from_millis(35));
    probe.update(&0);
    assert_eq!(sink.count(), 1);
}

#[test]
fn exporter_noop_is_hot_path_ok() {
    // Smoke-test that using `NoopExporter` with a real sampler
    // config works — the sampler still emits (to nowhere). This
    // is a legitimate deployment when a host wants the hot-path
    // cost but not the output.
    use sketch_runtime::exporter::NoopExporter;
    let sampler = Sampler::every_n(4, 1, NoopExporter, tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
    for i in 0..4 {
        probe.update(&i);
    }
}

// Additional probe to catch any panic / leak across the
// lifetime of the sampler when dropped mid-window.
#[test]
fn drop_mid_window_is_safe() {
    let sink = RecordSink::new();
    let sampler = Sampler::every_n(10, 1000, sink.clone_shared(), tag());
    let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
    for i in 0..5 {
        probe.update(&i);
    }
    drop(probe);
    // No window was closed, so no record was emitted.
    assert_eq!(sink.count(), 0);
}

#[test]
fn metrics_sink_blanket_impl_works_through_and() {
    // The `&mut T: MetricsSink` blanket impl in aqpbm-core
    // means Probe can wrap a sampler behind a mutable ref —
    // check the chain compiles + runs.
    //
    // With sample_every_n=2 + samples_per_window=2, each flush
    // needs 2 sampled ops ≡ 4 total ops. Drive the probe for 4
    // ops; expect exactly one record.
    let sink = RecordSink::new();
    let mut sampler = Sampler::every_n(2, 2, sink.clone_shared(), tag());
    {
        let mut probe: Probe<DummySketch, &mut Sampler<RecordSink>> =
            Probe::new(DummySketch { n: 0 }, &mut sampler);
        for i in 0..4 {
            probe.update(&i);
        }
    }
    assert_eq!(sink.count(), 1);
}
