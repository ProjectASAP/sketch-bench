//! What the parallel-insert rows share: the compiled-in matrix shape they
//! all run at, and the partition helper. Each row itself lives in its
//! algorithm's `sketchlib.rs`.

use asap_sketchlib::impl_fixed_matrix;

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// The shape every worker's matrix is baked at, and the `lg_k` its HLL is fixed
/// at. Written here because these rows *check* the request against them: the
/// per-worker sketch is a compile-time type, so any other config is unbuildable
/// and is refused instead of being accepted and ignored.
pub const PARALLEL_ROWS: usize = 5;

pub const PARALLEL_COLS: usize = 32768;

pub const PARALLEL_HLL_LG_K: u8 = 14;




pub fn partition(items: &[i64], n: usize) -> Vec<&[i64]> {
    let n = n.max(1);
    let chunk = (items.len() + n - 1) / n;
    if chunk == 0 {
        return vec![items];
    }
    items.chunks(chunk).collect()
}




