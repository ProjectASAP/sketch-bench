//! Nitro-style sketches — `sketchlib::NitroBatch<Vector2D>`
//! and `sketch_oxide::NitroSketch<CountMinSketch>`.
//!
//! Nitro is a sampling frequency-family sketch. For this
//! wrapper the `update` path is the sampled insert; `query`
//! returns a point-estimate like CMS.

use asap_sketchlib::{NitroBatch, Vector2D};
use sketch_core::sketch::Sketch;

use crate::params::{CMS_COLS, CMS_DELTA, CMS_EPSILON, CMS_ROWS, NITRO_RATE};

// ---------- asap_sketchlib NitroBatch ----------
//
// The legacy binary used `run_benchmark_i64_batch` because
// NitroBatch ingests via a batch `insert(values: &[i64])`. The
// `Sketch` trait takes one item at a time; we adapt by inserting
// single-element slices, which matches the trait shape but loses
// Nitro's batch optimisation. A dedicated batch-mode runner can
// be added later (tracked in TODO.md).
pub struct NitroLib {
    inner: NitroBatch<Vector2D<u32>>,
}

impl NitroLib {
    pub fn new() -> Self {
        let mut sk = Vector2D::<u32>::init(CMS_ROWS, CMS_COLS);
        sk.fill(0_u32);
        let inner = NitroBatch::with_target(NITRO_RATE, sk);
        Self { inner }
    }
}

impl Sketch for NitroLib {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.inner.insert(std::slice::from_ref(v));
    }
    fn bulk_update(&mut self, vs: &[i64]) {
        self.inner.insert(vs);
    }
    fn query(&self, _q: i64) -> u64 {
        // NitroBatch doesn't expose a cheap point-query in the
        // benched API; we return 0 for now — accuracy
        // comparisons for Nitro need a dedicated comparator
        // (future work).
        0
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<u32>()
    }
}

// ---------- sketch_oxide NitroSketch<CountMinSketch> ----------
pub struct NitroOxide(
    pub sketch_oxide::frequency::NitroSketch<sketch_oxide::frequency::CountMinSketch>,
);

impl NitroOxide {
    pub fn new() -> Self {
        let base = sketch_oxide::frequency::CountMinSketch::new(CMS_EPSILON, CMS_DELTA)
            .expect("valid CMS parameters");
        Self(
            sketch_oxide::frequency::NitroSketch::new(base, NITRO_RATE)
                .expect("valid Nitro parameters"),
        )
    }
}

impl Sketch for NitroOxide {
    type Item = Vec<u8>;
    type Query = Vec<u8>;
    type Answer = u64;
    fn update(&mut self, v: &Vec<u8>) {
        self.0.update_sampled(v);
    }
    fn query(&self, q: Vec<u8>) -> u64 {
        self.0.query(&q)
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<u64>()
    }
}
