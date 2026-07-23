//! Nitro-style sketches — `sketchlib::NitroBatch<Vector2D>`
//! and `sketch_oxide::NitroSketch<CountMinSketch>`.
//!
//! Nitro is a sampling frequency-family sketch. For this
//! wrapper the `update` path is the sampled insert; `query`
//! returns a point-estimate like CMS.
//!
//! The tunable param is `rate`; the underlying CMS is sized at
//! a fixed (5, 2048) — future work can expose those knobs too.

use crate::init::{BuildError, InitSketch};
use crate::params::NitroParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;
use asap_sketchlib::{NitroBatch, Vector2D};

const NITRO_CMS_ROWS: usize = 5;
const NITRO_CMS_COLS: usize = 2048;

// ---------- asap_sketchlib NitroBatch ----------
pub struct NitroLib {
    inner: NitroBatch<Vector2D<u32>>,
}

impl InitSketch for NitroLib {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: NitroParams = config.parse()?;
        let mut sk = Vector2D::<u32>::init(NITRO_CMS_ROWS, NITRO_CMS_COLS);
        sk.fill(0_u32);
        let inner = NitroBatch::with_target(p.rate, sk);
        Ok(Self { inner })
    }
}

impl Sketch for NitroLib {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(std::slice::from_ref(v));
    }
    fn bulk_update(&mut self, vs: &[i64]) {
        self.inner.insert(vs);
    }
    fn query(&self, _q: i64) -> u64 {
        // NitroBatch doesn't expose a cheap point-query in the
        // benched API; see wrapper notes in the older version.
        0
    }
    fn memory_bytes(&self) -> usize {
        NITRO_CMS_ROWS * NITRO_CMS_COLS * std::mem::size_of::<u32>()
    }
}

// ---------- sketch_oxide NitroSketch<CountMinSketch> ----------
pub struct NitroOxide(
    pub sketch_oxide::frequency::NitroSketch<sketch_oxide::frequency::CountMinSketch>,
);

impl InitSketch for NitroOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: NitroParams = config.parse()?;
        let epsilon = std::f64::consts::E / NITRO_CMS_COLS as f64;
        let delta = (-(NITRO_CMS_ROWS as f64)).exp();
        let base = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
            .map_err(|e| BuildError(format!("oxide CMS (nitro base) rejected: {e:?}")))?;
        let inner = sketch_oxide::frequency::NitroSketch::new(base, p.rate)
            .map_err(|e| BuildError(format!("oxide Nitro rejected rate={}: {e:?}", p.rate)))?;
        Ok(Self(inner))
    }
}

impl Sketch for NitroOxide {
    type Item = Vec<u8>;
    type Query = Vec<u8>;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &Vec<u8>) {
        self.0.update_sampled(v);
    }
    fn query(&self, q: Vec<u8>) -> u64 {
        self.0.query(&q)
    }
    fn memory_bytes(&self) -> usize {
        NITRO_CMS_ROWS * NITRO_CMS_COLS * std::mem::size_of::<u64>()
    }
}
