//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All three expose cardinality as
//! `Answer = f64`, `Query = ()`.

use sketch_core::sketch::Sketch;
// sketch_oxide routes `.estimate()` through its `Sketch` trait.
use sketch_oxide::Sketch as OxideSketch;

use crate::params::HLL_PRECISION;

// ---------- sketch_oxide HLL ----------
pub struct HllOxide(pub sketch_oxide::cardinality::HyperLogLog);

impl HllOxide {
    pub fn new() -> Self {
        Self(
            sketch_oxide::cardinality::HyperLogLog::new(HLL_PRECISION)
                .expect("valid HLL precision"),
        )
    }
}

impl Sketch for HllOxide {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn query(&self, _: ()) -> f64 {
        self.0.estimate()
    }
    fn memory_bytes(&self) -> usize {
        1usize << HLL_PRECISION
    }
}

// ---------- datasketches HLL ----------
pub struct HllDatasketches(pub datasketches::hll::HllSketch);

impl HllDatasketches {
    pub fn new() -> Self {
        Self(datasketches::hll::HllSketch::new(
            HLL_PRECISION,
            datasketches::hll::HllType::Hll8,
        ))
    }
}

impl Sketch for HllDatasketches {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.0.update(*v);
    }
    fn query(&self, _: ()) -> f64 {
        self.0.estimate()
    }
    fn memory_bytes(&self) -> usize {
        1usize << HLL_PRECISION
    }
}

// ---------- asap_sketchlib HLL ----------
// P14 ≙ precision 14 (same as `HLL_PRECISION` above). The lib's
// generic HyperLogLog<Variant> defaults to P14.
pub struct HllLib(pub asap_sketchlib::HyperLogLog<asap_sketchlib::ErtlMLE>);

impl HllLib {
    pub fn new() -> Self {
        Self(asap_sketchlib::HyperLogLog::<asap_sketchlib::ErtlMLE>::new())
    }
}

impl Sketch for HllLib {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.0.insert(&asap_sketchlib::DataInput::I64(*v));
    }
    fn query(&self, _: ()) -> f64 {
        self.0.estimate() as f64
    }
    fn memory_bytes(&self) -> usize {
        1usize << HLL_PRECISION
    }
}
