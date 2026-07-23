//! UnivMon wrappers — `sketchlib::UnivMon` (string-keyed) and
//! `sketch_oxide::universal::UnivMon` (byte-keyed). Multi-query
//! sketch: we treat "query" as a single scalar moment estimate
//! for Sketch-trait purposes; dedicated moment-family comparators
//! are a future-work item.
//!
//! The tunable params are `layers` and `max_stream`; the
//! underlying CMS is sized at a fixed (5, 2048).

use crate::init::{BuildError, InitSketch};
use crate::params::UnivMonParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;

const UNIVMON_CMS_ROWS: usize = 5;
const UNIVMON_CMS_COLS: usize = 2048;

// ---------- asap_sketchlib UnivMon ----------
pub struct UnivMonLib {
    inner: asap_sketchlib::UnivMon,
    layers: usize,
}

impl InitSketch for UnivMonLib {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: UnivMonParams = config.parse()?;
        Ok(Self {
            inner: asap_sketchlib::UnivMon::init_univmon(
                p.max_stream as usize,
                UNIVMON_CMS_ROWS,
                UNIVMON_CMS_COLS,
                p.layers,
            ),
            layers: p.layers,
        })
    }
}

impl Sketch for UnivMonLib {
    type Item = String;
    type Query = ();
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &String) {
        self.inner
            .fast_insert(&asap_sketchlib::DataInput::Str(v), 1);
    }
    fn query(&self, _: ()) -> f64 {
        0.0
    }
    fn memory_bytes(&self) -> usize {
        self.layers * UNIVMON_CMS_ROWS * UNIVMON_CMS_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- sketch_oxide UnivMon ----------
pub struct UnivMonOxide {
    inner: sketch_oxide::universal::UnivMon,
    layers: usize,
}

impl InitSketch for UnivMonOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: UnivMonParams = config.parse()?;
        let epsilon = std::f64::consts::E / UNIVMON_CMS_COLS as f64;
        let delta = (-(UNIVMON_CMS_ROWS as f64)).exp();
        let inner = sketch_oxide::universal::UnivMon::new(p.max_stream, epsilon, delta)
            .map_err(|e| BuildError(format!("oxide UnivMon rejected: {e:?}")))?;
        Ok(Self {
            inner,
            layers: p.layers,
        })
    }
}

impl Sketch for UnivMonOxide {
    type Item = Vec<u8>;
    type Query = ();
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.inner.update(v, 1.0).expect("UnivMon update succeeds");
    }
    fn query(&self, _: ()) -> f64 {
        0.0
    }
    fn memory_bytes(&self) -> usize {
        self.layers * UNIVMON_CMS_ROWS * UNIVMON_CMS_COLS * std::mem::size_of::<u64>()
    }
}
