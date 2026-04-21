//! UnivMon wrappers — `sketchlib::UnivMon` (string-keyed) and
//! `sketch_oxide::universal::UnivMon` (byte-keyed). Multi-query
//! sketch: we treat "query" as a single scalar moment estimate
//! for Sketch-trait purposes; dedicated moment-family comparators
//! are a future-work item.

use sketch_core::sketch::Sketch;

use crate::params::{
    CMS_COLS, CMS_DELTA, CMS_EPSILON, CMS_ROWS, UNIVMON_LAYERS, UNIVMON_MAX_STREAM,
};

// ---------- asap_sketchlib UnivMon ----------
pub struct UnivMonLib(pub asap_sketchlib::UnivMon);

impl UnivMonLib {
    pub fn new() -> Self {
        Self(asap_sketchlib::UnivMon::init_univmon(
            UNIVMON_MAX_STREAM as usize,
            CMS_ROWS,
            CMS_COLS,
            UNIVMON_LAYERS,
        ))
    }
}

impl Sketch for UnivMonLib {
    type Item = String;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &String) {
        self.0.fast_insert(&asap_sketchlib::DataInput::Str(v), 1);
    }
    fn query(&self, _: ()) -> f64 {
        // UnivMon queries: Fn(G) over L layers. A general
        // comparator lives in future work; for now return 0.
        0.0
    }
    fn memory_bytes(&self) -> usize {
        UNIVMON_LAYERS * CMS_ROWS * CMS_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- sketch_oxide UnivMon ----------
pub struct UnivMonOxide(pub sketch_oxide::universal::UnivMon);

impl UnivMonOxide {
    pub fn new() -> Self {
        Self(
            sketch_oxide::universal::UnivMon::new(UNIVMON_MAX_STREAM, CMS_EPSILON, CMS_DELTA)
                .expect("valid UnivMon params"),
        )
    }
}

impl Sketch for UnivMonOxide {
    type Item = Vec<u8>;
    type Query = ();
    type Answer = f64;
    fn update(&mut self, v: &Vec<u8>) {
        self.0.update(v, 1.0).expect("UnivMon update succeeds");
    }
    fn query(&self, _: ()) -> f64 {
        0.0
    }
    fn memory_bytes(&self) -> usize {
        UNIVMON_LAYERS * CMS_ROWS * CMS_COLS * std::mem::size_of::<u64>()
    }
}
