//! Elastic sketch wrappers — `sketchlib` (string-keyed) and
//! `oxide` (byte-keyed).

use std::cell::RefCell;

use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::ElasticParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;

// ---------- asap_sketchlib Elastic ----------
//
// The `RefCell` is vestigial: it existed so `Elastic::query`, which takes
// `&mut self` (interior state update on read), could be reached from behind a
// `&self` query method. This row declares no query capability, so the cell is
// now only ever reached through `get_mut` and could be a plain field.
//
// The lib's constructor only accepts `buckets`; `depth` is fixed internally,
// so the config records both but `depth` has no effect on the build.
pub struct ElasticLib {
    inner: RefCell<asap_sketchlib::Elastic<asap_sketchlib::DefaultXxHasher>>,
    buckets: usize,
}

impl InitSketch for ElasticLib {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: ElasticParams = config.parse()?;
        Ok(Self {
            inner: RefCell::new(
                asap_sketchlib::Elastic::<asap_sketchlib::DefaultXxHasher>::init_with_length(
                    p.buckets as i32,
                ),
            ),
            buckets: p.buckets,
        })
    }
}

impl Sketch for ElasticLib {
    type Item = String;
    #[inline(always)]
    fn update(&mut self, v: &String) {
        self.inner.get_mut().insert(v.clone());
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

impl InitSketch for ElasticOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: ElasticParams = config.parse()?;
        let inner =
            sketch_oxide::frequency::ElasticSketch::new(p.buckets, p.depth).map_err(|e| {
                BuildError(format!(
                    "oxide Elastic rejected buckets={} depth={}: {e:?}",
                    p.buckets, p.depth
                ))
            })?;
        Ok(Self {
            inner,
            buckets: p.buckets,
            depth: p.depth,
        })
    }
}

impl Sketch for ElasticOxide {
    type Item = Vec<u8>;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.inner.update(v, 1);
    }
    fn memory_bytes(&self) -> usize {
        self.buckets * self.depth * std::mem::size_of::<u64>()
    }
}

// ---------- catalog identity ----------

impl BenchImpl for ElasticLib { type Params = ElasticParams; const IMPL: &'static str = "lib"; }
impl BenchImpl for ElasticOxide { type Params = ElasticParams; const IMPL: &'static str = "oxide"; }
