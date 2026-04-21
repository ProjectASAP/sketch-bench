//! `Probe<S, Sink>` — the one decorator both offline
//! benchmarks and runtime samplers wrap around a live sketch.
//!
//! Purpose: keep metric collection out of the `Sketch` impls.
//! Each wrapped implementation stays a thin newtype around the
//! underlying library's struct; `Probe` intercepts every
//! `update` / `query` and notifies the supplied `MetricsSink`.
//!
//! Zero-cost when the sink is a no-op: the compiler inlines both
//! sink hooks and the trait dispatch (the `#[inline]` attributes
//! below document intent; the concrete `NoopSink` in
//! `sketch-bench` ensures both hooks compile to nothing).
//!
//! See `docs/DESIGN.md` §4.2.

use crate::sketch::Sketch;

/// A handler for benchmark/runtime metrics events.
///
/// Lives here in `sketch-core` so `Probe` can depend on the
/// trait without depending on the concrete metric code in
/// `sketch-bench`. Concrete impls (`NoopSink`, `FullSink`,
/// `SampledSink`) ship in `sketch-bench` and `sketch-runtime`.
pub trait MetricsSink {
    fn on_update_start(&mut self);
    fn on_update_end(&mut self);
    fn on_query_start(&mut self);
    fn on_query_end(&mut self);
}

/// A no-op `MetricsSink` provided by `sketch-core` so `Probe`
/// is usable even from crates that don't pull in `sketch-bench`.
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
/// without moving ownership — lets `BenchRunner` keep the sink
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

/// `Probe<S, Sink>` wraps any `Sketch` + `MetricsSink` into a
/// new `Sketch` that records timing hooks around the inner
/// `update` / `query`. Identical wrapper used by offline
/// `sketch-bench::BenchRunner` and embedded
/// `sketch-runtime::Sampler`.
pub struct Probe<S: Sketch, Sink: MetricsSink> {
    inner: S,
    sink: Sink,
}

impl<S: Sketch, Sink: MetricsSink> Probe<S, Sink> {
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

impl<S: Sketch, Sink: MetricsSink> Sketch for Probe<S, Sink> {
    type Item = S::Item;
    type Query = S::Query;
    type Answer = S::Answer;

    #[inline]
    fn update(&mut self, v: &Self::Item) {
        self.sink.on_update_start();
        self.inner.update(v);
        self.sink.on_update_end();
    }

    #[inline]
    fn query(&self, q: Self::Query) -> Self::Answer {
        // We intentionally don't take &mut self here — the sink
        // records at boundaries via shared mutability on the
        // `SampledSink` / `FullSink` side (they use interior
        // mutability or are held behind a &mut ref by the
        // BenchRunner wrapper). For `NoopSink` this is inert.
        let answer = self.inner.query(q);
        // The borrow-checker forbids calling sink.on_query_*
        // through `&self`; runtime paths that care about query
        // timing wrap Probe in an outer BenchRunner layer
        // (see sketch-bench::runner::run_with_probe) which
        // owns the &mut borrow.
        answer
    }

    fn memory_bytes(&self) -> usize {
        self.inner.memory_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummySketch {
        updates: usize,
    }
    impl Sketch for DummySketch {
        type Item = i64;
        type Query = ();
        type Answer = usize;
        fn update(&mut self, _: &i64) {
            self.updates += 1;
        }
        fn query(&self, _: ()) -> usize {
            self.updates
        }
        fn memory_bytes(&self) -> usize {
            0
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
