//! Elastic sketch wrappers — `sketchlib` (string-keyed) and
//! `oxide` (byte-keyed).

use std::cell::RefCell;

use crate::params::ElasticParams;
use aqpbm_core::sketch::Sketch;

// ---------- asap_sketchlib Elastic ----------
//
// `Elastic::query` takes `&mut self` (interior state update on
// read) and `String` by value. We wrap in `RefCell` so the
// `Sketch` trait's `fn query(&self, ...)` contract still holds;
// interior mutability is safe here because BenchRunner queries
// serially per run.
//
// The lib's constructor only accepts `buckets`; `depth` is fixed
// internally, so we honour `buckets` and record both in the
// sweep's JSONL config but `depth` has no effect on the build.
pub struct ElasticLib {
    inner: RefCell<asap_sketchlib::Elastic<asap_sketchlib::DefaultXxHasher>>,
    buckets: usize,
}

impl ElasticLib {
    pub fn new(p: &ElasticParams) -> Self {
        Self {
            inner: RefCell::new(
                asap_sketchlib::Elastic::<asap_sketchlib::DefaultXxHasher>::init_with_length(
                    p.buckets as i32,
                ),
            ),
            buckets: p.buckets,
        }
    }
}

impl Sketch for ElasticLib {
    type Item = String;
    type Query = String;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &String) {
        self.inner.get_mut().insert(v.clone());
    }
    fn query(&self, q: String) -> u64 {
        self.inner.borrow_mut().query(q).max(0) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.buckets * std::mem::size_of::<u32>() * 4
    }
}

// ---------- sketch_oxide ElasticSketch ----------
pub struct ElasticOxide {
    inner: sketch_oxide::frequency::ElasticSketch,
    buckets: usize,
    depth: usize,
}

impl ElasticOxide {
    pub fn new(p: &ElasticParams) -> Self {
        Self {
            inner: sketch_oxide::frequency::ElasticSketch::new(p.buckets, p.depth)
                .expect("valid Elastic parameters"),
            buckets: p.buckets,
            depth: p.depth,
        }
    }
}

impl Sketch for ElasticOxide {
    type Item = Vec<u8>;
    type Query = Vec<u8>;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.inner.update(v, 1);
    }
    fn query(&self, q: Vec<u8>) -> u64 {
        self.inner.estimate(&q)
    }
    fn memory_bytes(&self) -> usize {
        self.buckets * self.depth * std::mem::size_of::<u64>()
    }
}
