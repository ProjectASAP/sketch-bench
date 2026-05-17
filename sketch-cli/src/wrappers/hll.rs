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
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate()
    }
    fn memory_bytes(&self) -> usize {
        // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
        1usize << self.lg_k
    }
}

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: datasketches::hll::HllType,
}

impl HllDatasketches {
    pub fn new(p: &HllParams) -> Self {
        let hll_type = datasketches::hll::HllType::Hll8;
        Self {
            inner: datasketches::hll::HllSketch::new(p.lg_k, hll_type),
            lg_k: p.lg_k,
            hll_type,
        }
    }
}

impl Sketch for HllDatasketches {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate()
    }
    fn memory_bytes(&self) -> usize {
        // Apache datasketches packs registers per HllType:
        // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
        let m = 1usize << self.lg_k;
        match self.hll_type {
            datasketches::hll::HllType::Hll4 => m / 2,
            datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
            datasketches::hll::HllType::Hll8 => m,
        }
    }
}

// ---------- asap_sketchlib HLL ----------
// `asap_sketchlib::HyperLogLog<Classic>` is the P14 classic HLL estimator
// (Flajolet et al., 2007). Insert path only bumps registers; `estimate()`
// scans all 2^14 registers (O(m)). Compile-time fixed at P14; the
// requested `lg_k` is ignored.
pub struct HllLib {
    inner: asap_sketchlib::HyperLogLog<asap_sketchlib::Classic>,
}

impl HllLib {
    pub fn new(_p: &HllParams) -> Self {
        Self {
            inner: asap_sketchlib::HyperLogLog::<asap_sketchlib::Classic>::new(),
        }
    }
}

impl Sketch for HllLib {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
    fn query(&self, _: ()) -> f64 {
        self.inner.estimate() as f64
    }
    fn memory_bytes(&self) -> usize {
        // Implementation is fixed at P14 — register count is 2^14
        // regardless of `HllParams::lg_k`, 1 byte per register.
        1usize << 14
    }
}
