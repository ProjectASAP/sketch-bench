//! CountSketch wrappers — 4 variants (`oxide` + 3× sketchlib).
//! Same family as CMS: `Query = i64`, `Answer = u64`.

use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D};
use sketch_core::config::CountSketchParams;
use sketch_core::sketch::Sketch;

use crate::wrappers::cms::{CMS_FIXED_COLS, CMS_FIXED_ROWS};

fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = std::f64::consts::E / cols as f64;
    let delta = (-(rows as f64)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide CountSketch ----------
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
    rows: usize,
    cols: usize,
}

impl CsOxide {
    pub fn new(p: &CountSketchParams) -> Self {
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        Self {
            inner: sketch_oxide::frequency::CountSketch::new(epsilon, delta)
                .expect("valid CountSketch parameters"),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsOxide {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.inner.update(v, 1);
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&q) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
// Compile-time fixed at (5, 2048). Dispatch marks this as
// fixed-shape; sweep runs it only when requested shape matches.
pub struct CsLibFixedmatrixFast(pub Count<FixedMatrix, FastPath>);

impl CsLibFixedmatrixFast {
    pub fn new(_p: &CountSketchParams) -> Self {
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
        CMS_FIXED_ROWS * CMS_FIXED_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CsLibVector2dFast {
    inner: Count<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

impl CsLibVector2dFast {
    pub fn new(p: &CountSketchParams) -> Self {
        Self {
            inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsLibVector2dFast {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CsLibVector2dRegular {
    inner: Count<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

impl CsLibVector2dRegular {
    pub fn new(p: &CountSketchParams) -> Self {
        Self {
            inner: Count::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsLibVector2dRegular {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}
