//! CountSketch wrappers — 4 variants (`oxide` + 3× sketchlib).
//! Same family as CMS: `Query = i64`, `Answer = u64`.

use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D};
use sketch_core::sketch::Sketch;

use crate::params::{CMS_COLS, CMS_DELTA, CMS_EPSILON, CMS_ROWS};

// ---------- sketch_oxide CountSketch ----------
pub struct CsOxide(pub sketch_oxide::frequency::CountSketch);

impl CsOxide {
    pub fn new() -> Self {
        Self(
            sketch_oxide::frequency::CountSketch::new(CMS_EPSILON, CMS_DELTA)
                .expect("valid CountSketch parameters"),
        )
    }
}

impl Sketch for CsOxide {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.update(v, 1);
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&q) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<i64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
pub struct CsLibFixedmatrixFast(pub Count<FixedMatrix, FastPath>);

impl CsLibFixedmatrixFast {
    pub fn new() -> Self {
        Self(Count::<FixedMatrix, FastPath>::default())
    }
}

impl Sketch for CsLibFixedmatrixFast {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CsLibVector2dFast(pub Count<Vector2D<i32>, FastPath>);

impl CsLibVector2dFast {
    pub fn new() -> Self {
        Self(Count::<Vector2D<i32>, FastPath>::with_dimensions(
            CMS_ROWS, CMS_COLS,
        ))
    }
}

impl Sketch for CsLibVector2dFast {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CsLibVector2dRegular(pub Count<Vector2D<i32>, RegularPath>);

impl CsLibVector2dRegular {
    pub fn new() -> Self {
        Self(Count::<Vector2D<i32>, RegularPath>::with_dimensions(
            CMS_ROWS, CMS_COLS,
        ))
    }
}

impl Sketch for CsLibVector2dRegular {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<i32>()
    }
}
