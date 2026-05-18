//! DDSketch wrapper: `asap_sketchlib::DDSketch`.
//! Quantile family with relative-error guarantee `alpha`:
//! `Query = f64` (quantile in [0, 1]), `Answer = f64` (value
//! at quantile).

use asap_sketchlib::DDSketch;
use sketch_core::config::DdParams;
use sketch_core::sketch::Sketch;

pub struct DdLib {
    inner: DDSketch,
}

impl DdLib {
    pub fn new(p: &DdParams) -> Self {
        Self {
            inner: DDSketch::new(p.alpha),
        }
    }
}

impl Sketch for DdLib {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.add(&(*v as f64));
    }
    fn query(&self, q: f64) -> f64 {
        self.inner.get_value_at_quantile(q).unwrap_or(f64::NAN)
    }
    fn memory_bytes(&self) -> usize {
        // Best-effort estimate: DDSketch's bucket store is
        // dynamically sized; fall back to the serialised size.
        self.inner
            .serialize_to_bytes()
            .map(|b| b.len())
            .unwrap_or(0)
    }
}
