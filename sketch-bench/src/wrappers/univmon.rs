//! UnivMon wrappers — `sketchlib::UnivMon` (string-keyed) and
//! `sketch_oxide::universal::UnivMon` (byte-keyed). Neither declares a query
//! capability, so neither is accuracy-scored; moment-algorithm comparators are
//! future work. Tunable params are `layers` and `max_stream`, over a fixed
//! (5, 2048) CMS — and oxide's constructor ignores `layers` beyond footprint.

use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::UnivMonParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::memory_footprint::MemoryFootprint;

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

impl Accumulator for UnivMonLib {
    type Item = String;
    #[inline(always)]
    fn update(&mut self, v: &String) {
        self.inner
            .fast_insert(&asap_sketchlib::DataInput::Str(v), 1);
    }
}

impl MemoryFootprint for UnivMonLib {
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

impl Accumulator for UnivMonOxide {
    type Item = Vec<u8>;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.inner.update(v, 1.0).expect("UnivMon update succeeds");
    }
}

impl MemoryFootprint for UnivMonOxide {
    fn memory_bytes(&self) -> usize {
        self.layers * UNIVMON_CMS_ROWS * UNIVMON_CMS_COLS * std::mem::size_of::<u64>()
    }
}

// ---------- catalog identity ----------

impl BenchImpl for UnivMonLib { type Params = UnivMonParams; const IMPL: &'static str = "lib"; }
impl BenchImpl for UnivMonOxide { type Params = UnivMonParams; const IMPL: &'static str = "oxide"; }
