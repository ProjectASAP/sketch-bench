//! Count-Min Sketch wrappers — 6 variants.
//!
//! Frequency family: `Query = i64` (point lookup),
//! `Answer = u64` (count estimate).
//!
//! Impls:
//! * `oxide` — sketch_oxide::frequency::CountMinSketch
//! * `datasketches` — datasketches::countmin::CountMinSketch
//! * `lib_fixedmatrix_custom_fast` — asap_sketchlib custom storage
//! * `lib_fixedmatrix_fast` — asap_sketchlib FixedMatrix + FastPath
//! * `lib_vector2d_fast` — asap_sketchlib Vector2D + FastPath
//! * `lib_vector2d_regular` — asap_sketchlib Vector2D + RegularPath

use sketch_core::sketch::Sketch;

use asap_sketchlib::{
    impl_fixed_matrix, CountMin, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D,
};

use crate::params::{CMS_COLS, CMS_DELTA, CMS_EPSILON, CMS_ROWS};

// ---------- sketch_oxide ----------
pub struct CmsOxide(pub sketch_oxide::frequency::CountMinSketch);

impl CmsOxide {
    pub fn new() -> Self {
        Self(
            sketch_oxide::frequency::CountMinSketch::new(CMS_EPSILON, CMS_DELTA)
                .expect("valid CMS parameters"),
        )
    }
}

impl Sketch for CmsOxide {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&q)
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<u32>()
    }
}

// ---------- datasketches ----------
pub struct CmsDatasketches(pub datasketches::countmin::CountMinSketch);

impl CmsDatasketches {
    pub fn new() -> Self {
        Self(datasketches::countmin::CountMinSketch::new(
            CMS_ROWS as u8,
            CMS_COLS as u32,
        ))
    }
}

impl Sketch for CmsDatasketches {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    fn update(&mut self, v: &i64) {
        self.0.update(*v);
    }
    fn query(&self, q: i64) -> u64 {
        // `estimate<T: Hash>` is a by-value API in datasketches.
        self.0.estimate(q).max(0) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_ROWS * CMS_COLS * std::mem::size_of::<u64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix custom + FastPath ----------
impl_fixed_matrix!(CustomCountMinMatrixI32U128, i32, 5, 65538);

pub struct CmsLibFixedmatrixCustomFast(pub CountMin<CustomCountMinMatrixI32U128, FastPath>);

impl CmsLibFixedmatrixCustomFast {
    pub fn new() -> Self {
        Self(
            CountMin::<CustomCountMinMatrixI32U128, FastPath>::from_storage(
                CustomCountMinMatrixI32U128::default(),
            ),
        )
    }
}

impl Sketch for CmsLibFixedmatrixCustomFast {
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
        5 * 65538 * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
pub struct CmsLibFixedmatrixFast(pub CountMin<FixedMatrix, FastPath>);

impl CmsLibFixedmatrixFast {
    pub fn new() -> Self {
        Self(CountMin::<FixedMatrix, FastPath>::default())
    }
}

impl Sketch for CmsLibFixedmatrixFast {
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
        // FixedMatrix's internal dimensions aren't exposed; the
        // default matches the defaults the binaries use.
        CMS_ROWS * CMS_COLS * std::mem::size_of::<u32>()
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CmsLibVector2dFast(pub CountMin<Vector2D<i32>, FastPath>);

impl CmsLibVector2dFast {
    pub fn new() -> Self {
        Self(CountMin::<Vector2D<i32>, FastPath>::with_dimensions(
            CMS_ROWS, CMS_COLS,
        ))
    }
}

impl Sketch for CmsLibVector2dFast {
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
pub struct CmsLibVector2dRegular(pub CountMin<Vector2D<i32>, RegularPath>);

impl CmsLibVector2dRegular {
    pub fn new() -> Self {
        Self(CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(
            CMS_ROWS, CMS_COLS,
        ))
    }
}

impl Sketch for CmsLibVector2dRegular {
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
