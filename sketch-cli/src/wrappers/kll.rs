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

use sketch_core::config::KllParams;
use sketch_core::sketch::Sketch;

// ---------- sketch_oxide KLL ----------
// `KllSketch::default()` constructs with the crate's built-in `k`
// (no `new(k)` constructor exposed through the stable surface).
// We store the requested `k` so `memory_bytes` is sensible; the
// sweep driver marks oxide KLL as fixed-shape in the dispatch.
pub struct KllOxide {
    inner: RefCell<sketch_oxide::quantiles::KllSketch>,
    k: u32,
}

impl KllOxide {
    pub fn new(p: &KllParams) -> Self {
        Self {
            inner: RefCell::new(sketch_oxide::quantiles::KllSketch::default()),
            k: p.k,
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
        (self.k as usize) * std::mem::size_of::<f64>() * 4
    }
}

// ---------- asap_sketchlib KLL ----------
pub struct KllLib {
    inner: asap_sketchlib::KLL<i64>,
    k: u32,
}

impl KllLib {
    pub fn new(p: &KllParams) -> Self {
        Self {
            inner: asap_sketchlib::KLL::<i64>::init_kll(p.k as i32),
            k: p.k,
        }
    }
}

impl Sketch for KllLib {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn query(&self, q: f64) -> f64 {
        self.inner.quantile(q)
    }
    fn memory_bytes(&self) -> usize {
        (self.k as usize) * std::mem::size_of::<i64>() * 4
    }
}
