//! Overhead of `Probe<_, Sampler>` vs `Probe<_, NoopSink>` at sampling rate
//! 1/1024; design doc §10 sets the ceiling at 1% throughput loss. Uses a trivial
//! counting "sketch" so the sink hook cost dominates, not the sketch.

use std::hint::black_box;

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::probe::{NoopSink, Probe};
use aqpbm_core::report::Source;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use sketch_runtime::exporter::NoopExporter;
use sketch_runtime::sampler::{Sampler, Tag};

struct DummySketch {
    n: u64,
}
impl Accumulator for DummySketch {
    type Item = i64;
    #[inline]
    fn update(&mut self, v: &i64) {
        self.n = self.n.wrapping_add(*v as u64);
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
