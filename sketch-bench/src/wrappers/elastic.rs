//! Elastic sketch wrappers — `sketchlib` (string-keyed) and
//! `oxide` (byte-keyed).

use std::cell::RefCell;

use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::ElasticParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::memory_footprint::MemoryFootprint;

// ---------- asap_sketchlib Elastic ----------
// The `RefCell` is vestigial: no query capability here, so it is only reached
// through `get_mut`. The constructor takes only `buckets`; `depth` is inert.
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

impl Accumulator for ElasticLib {
    type Item = String;
    #[inline(always)]
    fn update(&mut self, v: &String) {
        self.inner.get_mut().insert(v.clone());
    }
}

impl MemoryFootprint for ElasticLib {
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

impl Accumulator for ElasticOxide {
    type Item = Vec<u8>;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.inner.update(v, 1);
    }
}

impl MemoryFootprint for ElasticOxide {
    fn memory_bytes(&self) -> usize {
        self.buckets * self.depth * std::mem::size_of::<u64>()
    }
}

// ---------- catalog identity ----------

impl BenchImpl for ElasticLib { type Params = ElasticParams; const IMPL: &'static str = "lib"; }
impl BenchImpl for ElasticOxide { type Params = ElasticParams; const IMPL: &'static str = "oxide"; }
