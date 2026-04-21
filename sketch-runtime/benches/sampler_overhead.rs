//! Overhead of `Probe<_, Sampler>` vs `Probe<_, NoopSink>` at
//! sampling rate 1/1024. Design doc §10 sets the ceiling at 1%
//! throughput loss.
//!
//! Uses a trivial counting "sketch" so the measurement is
//! dominated by the sink hook cost, not the sketch itself.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use sketch_core::probe::{NoopSink, Probe};
use sketch_core::report::Source;
use sketch_core::sketch::Sketch;
use sketch_runtime::exporter::NoopExporter;
use sketch_runtime::sampler::{Sampler, Tag};

struct DummySketch {
    n: u64,
}
impl Sketch for DummySketch {
    type Item = i64;
    type Query = ();
    type Answer = u64;
    #[inline]
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

fn baseline_noop_sink(c: &mut Criterion) {
    let mut g = c.benchmark_group("probe_overhead");
    g.throughput(Throughput::Elements(1));
    g.bench_function("noop_sink", |b| {
        let mut probe = Probe::new(DummySketch { n: 0 }, NoopSink);
        b.iter(|| {
            probe.update(black_box(&1_i64));
        });
    });
    g.bench_function("sampler_every_1024", |b| {
        let tag = Tag::new("dummy", "bench", Source::DataCollector);
        let sampler = Sampler::every_n(1024, u32::MAX, NoopExporter, tag);
        let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
        b.iter(|| {
            probe.update(black_box(&1_i64));
        });
    });
    g.bench_function("sampler_disabled", |b| {
        let tag = Tag::new("dummy", "bench", Source::DataCollector);
        let sampler = Sampler::disabled(NoopExporter, tag);
        let mut probe = Probe::new(DummySketch { n: 0 }, sampler);
        b.iter(|| {
            probe.update(black_box(&1_i64));
        });
    });
    g.finish();
}

criterion_group!(benches, baseline_noop_sink);
criterion_main!(benches);
