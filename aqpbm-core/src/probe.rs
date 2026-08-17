//! `Probe<S, Sink>` — the one decorator both offline benchmarks and runtime
//! samplers wrap around a live sketch. It keeps metric collection out of the
//! `Accumulator` impls by intercepting every `update` to notify a `MetricsSink`.
//! With [`NoopSink`] the hooks and the dispatch inline away to nothing.
//! See `docs/DESIGN.md` §4.2.

use crate::accumulator::Accumulator;

/// A handler for benchmark/runtime metrics events. Impls: [`NoopSink`] below,
/// [`FullSink`](crate::metrics::FullSink) for offline runs, and
/// `sketch-runtime`'s `Sampler` for the embedded path.
pub trait MetricsSink {
    fn on_update_start(&mut self);
    fn on_update_end(&mut self);
    fn on_query_start(&mut self);
    fn on_query_end(&mut self);
}

/// A `MetricsSink` that records nothing — the zero-overhead default for
/// callers that want the `Probe` shape without any measurement.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl MetricsSink for NoopSink {
    #[inline(always)]
    fn on_update_start(&mut self) {}
    #[inline(always)]
    fn on_update_end(&mut self) {}
    #[inline(always)]
    fn on_query_start(&mut self) {}
    #[inline(always)]
    fn on_query_end(&mut self) {}
}

/// Blanket impl so callers can hand a `&mut Sink` into `Probe`
/// without moving ownership — lets a caller keep the sink
/// alive past the probe and call `finalize` on it.
impl<T: MetricsSink + ?Sized> MetricsSink for &mut T {
    #[inline]
    fn on_update_start(&mut self) {
        (**self).on_update_start()
    }
    #[inline]
    fn on_update_end(&mut self) {
        (**self).on_update_end()
    }
    #[inline]
    fn on_query_start(&mut self) {
        (**self).on_query_start()
    }
    #[inline]
    fn on_query_end(&mut self) {
        (**self).on_query_end()
    }
}

/// `Probe<S, Sink>` wraps an `Accumulator` + `MetricsSink` into an `Accumulator`
/// recording timing hooks around the inner `update`. The identical wrapper serves
/// the offline recorder and the embedded sampler.
pub struct Probe<S: Accumulator, Sink: MetricsSink> {
    inner: S,
    sink: Sink,
}

impl<S: Accumulator, Sink: MetricsSink> Probe<S, Sink> {
    pub fn new(inner: S, sink: Sink) -> Self {
        Self { inner, sink }
    }

    pub fn into_parts(self) -> (S, Sink) {
        (self.inner, self.sink)
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn sink_mut(&mut self) -> &mut Sink {
        &mut self.sink
    }
}

impl<S: Accumulator, Sink: MetricsSink> Accumulator for Probe<S, Sink> {
    type Item = S::Item;

    #[inline]
    fn update(&mut self, v: &Self::Item) {
        self.sink.on_update_start();
        self.inner.update(v);
        self.sink.on_update_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummySketch {
        updates: usize,
    }
    impl Accumulator for DummySketch {
        type Item = i64;
        fn update(&mut self, _: &i64) {
            self.updates += 1;
        }
    }

    #[derive(Default)]
    struct CountingSink {
        updates: usize,
    }
    impl MetricsSink for CountingSink {
        fn on_update_start(&mut self) {
            self.updates += 1;
        }
        fn on_update_end(&mut self) {}
        fn on_query_start(&mut self) {}
        fn on_query_end(&mut self) {}
    }

    #[test]
    fn probe_passes_updates_through_and_records_hooks() {
        let mut p = Probe::new(DummySketch { updates: 0 }, CountingSink::default());
        p.update(&1);
        p.update(&2);
        p.update(&3);
        assert_eq!(p.inner().updates, 3);
        assert_eq!(p.sink_mut().updates, 3);
    }

    #[test]
    fn probe_with_noop_sink_is_pass_through() {
        let mut p = Probe::new(DummySketch { updates: 0 }, NoopSink);
        p.update(&42);
        assert_eq!(p.inner().updates, 1);
        // NoopSink has no observable state — assertion is
        // compilation + no-panic.
    }
}
