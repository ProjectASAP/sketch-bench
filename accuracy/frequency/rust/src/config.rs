//! Shared constants for the frequency accuracy harness.
//!
//! CMS and Count-Sketch runners share the same `ROWS` /
//! `COLS_LIST` sweep but tag their output rows with family-specific
//! implementation names (`rust_oxide_cms` vs `rust_oxide_cs`, etc.)
//! so the plot scripts can group them.

pub mod cms_impl {
    pub const DATASKETCHES: &str = "rust_datasketches_cms";
    pub const OXIDE: &str = "rust_oxide_cms";
    pub const SKETCHLIB: &str = "rust_sketchlib_cms";
}

pub mod cs_impl {
    pub const OXIDE: &str = "rust_oxide_cs";
    pub const SKETCHLIB: &str = "rust_sketchlib_cs";
}

pub const ROWS: usize = 5;
pub const COLS_LIST: [usize; 7] = [2048, 4096, 8192, 16384, 32768, 65536, 131072];
