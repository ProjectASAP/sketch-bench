//! KLL wrappers: `oxide`, `sketchlib` (a.k.a. asap_sketchlib).
//! Quantile family: `Query = f64` (quantile in [0, 1]),
//! `Answer = f64` (value at quantile).
//!
//! Note: `sketch_oxide::quantiles::KllSketch::quantile` takes
//! `&mut self` (the sketch sorts lazily). We wrap the inner in
//! `RefCell` so the `Sketch::query(&self, ...)` contract still
//! works — the trait is intentionally `&self` since most
//! sketches' query paths are pure reads; KLL-oxide is the one
//! that isn't.

use std::cell::RefCell;

use sketch_core::sketch::Sketch;

use crate::params::KLL_K;

// ---------- sketch_oxide KLL ----------
pub struct KllOxide {
    inner: RefCell<sketch_oxide::quantiles::KllSketch>,
}

impl KllOxide {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(sketch_oxide::quantiles::KllSketch::default()),
        }
    }
}

impl Sketch for KllOxide {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.inner.get_mut().update(*v as f64);
    }
    fn query(&self, q: f64) -> f64 {
        self.inner.borrow_mut().quantile(q).unwrap_or(f64::NAN)
    }
    fn memory_bytes(&self) -> usize {
        (KLL_K as usize) * std::mem::size_of::<f64>() * 4
    }
}

// ---------- asap_sketchlib KLL ----------
pub struct KllLib(pub asap_sketchlib::KLL<i64>);

impl KllLib {
    pub fn new() -> Self {
        Self(asap_sketchlib::KLL::<i64>::init_kll(KLL_K))
    }
}

impl Sketch for KllLib {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn query(&self, q: f64) -> f64 {
        self.0.quantile(q)
    }
    fn memory_bytes(&self) -> usize {
        (KLL_K as usize) * std::mem::size_of::<i64>() * 4
    }
}
