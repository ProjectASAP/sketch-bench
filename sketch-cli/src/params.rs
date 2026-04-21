//! Shared sketch-construction defaults — the knobs that were
//! hard-coded across the 21 legacy binaries. Centralising them
//! here means the CLI has one source of truth and every wrapper
//! pulls from it.

pub const HLL_PRECISION: u8 = 14;
pub const CMS_EPSILON: f64 = 0.0013;
pub const CMS_DELTA: f64 = 0.0067;
pub const CMS_ROWS: usize = 5;
pub const CMS_COLS: usize = 2048;
pub const ELASTIC_BUCKETS: usize = 1024;
pub const ELASTIC_DEPTH: usize = 3;
pub const KLL_K: i32 = 200;
pub const UNIVMON_MAX_STREAM: u64 = 256;
pub const UNIVMON_LAYERS: usize = 8;
pub const NITRO_RATE: f64 = 0.01;
