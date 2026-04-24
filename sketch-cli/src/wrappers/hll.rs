//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All three expose cardinality as
//! `Answer = f64`, `Query = ()`.

use sketch_core::config::HllParams;
use sketch_core::sketch::Sketch;
// sketch_oxide routes `.estimate()` through its `Sketch` trait.
use sketch_oxide::Sketch as OxideSketch;

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

impl HllOxide {
    pub fn new(p: &HllParams) -> Self {
        Self {
            inner: sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
                .expect("valid HLL precision"),
            lg_k: p.lg_k,
        }
    }
}

impl Sketch for HllOxide {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate()
    }
    fn memory_bytes(&self) -> usize {
        1usize << self.lg_k
    }
}

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: datasketches::hll::HllSketch,
    lg_k: u8,
}

impl HllDatasketches {
    pub fn new(p: &HllParams) -> Self {
        Self {
            inner: datasketches::hll::HllSketch::new(p.lg_k, datasketches::hll::HllType::Hll8),
            lg_k: p.lg_k,
        }
    }
}

impl Sketch for HllDatasketches {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate()
    }
    fn memory_bytes(&self) -> usize {
        1usize << self.lg_k
    }
}

// ---------- asap_sketchlib HLL ----------
// `asap_sketchlib::HyperLogLog::new()` constructs at the lib's
// compile-time P14 default; it doesn't expose a runtime `lg_k`
// constructor through the current bindings. We store the requested
// `lg_k` so `memory_bytes` reports something sensible, and flag
// this as a fixed-shape impl in `dispatch::ImplEntry`.
pub struct HllLib {
    inner: asap_sketchlib::HyperLogLog<asap_sketchlib::ErtlMLE>,
    lg_k: u8,
}

impl HllLib {
    pub fn new(p: &HllParams) -> Self {
        Self {
            inner: asap_sketchlib::HyperLogLog::<asap_sketchlib::ErtlMLE>::new(),
            lg_k: p.lg_k,
        }
    }
}

impl Sketch for HllLib {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate() as f64
    }
    fn memory_bytes(&self) -> usize {
        1usize << self.lg_k
    }
}
