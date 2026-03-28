use crate::baseline::BaselineData;
use crate::config::{COLS_LIST, IMPLEMENTATION_RUST_SKETCHLIB, ROWS};
use crate::output::AccuracyRow;
use sketchlib_rust::{
    hash_mode_for_matrix, impl_fixed_matrix, CountMin, FastPath, HeapItem, MatrixHashType,
    MatrixStorage, SketchHasher, SketchInput,
};
use smallvec::SmallVec;
use twox_hash::{XxHash3_128, XxHash3_64};

impl_fixed_matrix!(M3x2K, i32, 3, 2048);
impl_fixed_matrix!(M3x4K, i32, 3, 4096);
impl_fixed_matrix!(M3x8K, i32, 3, 8192);

macro_rules! define_seeded_hasher {
    ($name:ident, $seed:expr) => {
        #[derive(Clone, Debug)]
        struct $name;

        impl SketchHasher for $name {
            type HashType = MatrixHashType;

            fn hash64_seeded(d: usize, key: &SketchInput) -> u64 {
                hash_input64($seed + d as u64, key)
            }

            fn hash128_seeded(d: usize, key: &SketchInput) -> u128 {
                hash_input128($seed + d as u64, key)
            }

            fn hash_item64_seeded(d: usize, key: &HeapItem) -> u64 {
                hash_heap64($seed + d as u64, key)
            }

            fn hash_item128_seeded(d: usize, key: &HeapItem) -> u128 {
                hash_heap128($seed + d as u64, key)
            }

            fn hash_for_matrix_seeded(
                _seed_idx: usize,
                rows: usize,
                _cols: usize,
                key: &SketchInput,
            ) -> Self::HashType {
                let mut hashes = SmallVec::<[u64; 8]>::with_capacity(rows);
                for row in 0..rows {
                    hashes.push(Self::hash64_seeded(row, key));
                }
                MatrixHashType::Rows(hashes)
            }
        }
    };
}

define_seeded_hasher!(H01, 1);
define_seeded_hasher!(H02, 2);
define_seeded_hasher!(H03, 3);
define_seeded_hasher!(H04, 4);
define_seeded_hasher!(H05, 5);
define_seeded_hasher!(H06, 6);
define_seeded_hasher!(H07, 7);
define_seeded_hasher!(H08, 8);
define_seeded_hasher!(H09, 9);
define_seeded_hasher!(H10, 10);

pub fn run(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(30);
    rows.push(run_one::<M3x2K, H01>(1, 2048, baseline));
    rows.push(run_one::<M3x4K, H01>(1, 4096, baseline));
    rows.push(run_one::<M3x8K, H01>(1, 8192, baseline));
    rows.push(run_one::<M3x2K, H02>(2, 2048, baseline));
    rows.push(run_one::<M3x4K, H02>(2, 4096, baseline));
    rows.push(run_one::<M3x8K, H02>(2, 8192, baseline));
    rows.push(run_one::<M3x2K, H03>(3, 2048, baseline));
    rows.push(run_one::<M3x4K, H03>(3, 4096, baseline));
    rows.push(run_one::<M3x8K, H03>(3, 8192, baseline));
    rows.push(run_one::<M3x2K, H04>(4, 2048, baseline));
    rows.push(run_one::<M3x4K, H04>(4, 4096, baseline));
    rows.push(run_one::<M3x8K, H04>(4, 8192, baseline));
    rows.push(run_one::<M3x2K, H05>(5, 2048, baseline));
    rows.push(run_one::<M3x4K, H05>(5, 4096, baseline));
    rows.push(run_one::<M3x8K, H05>(5, 8192, baseline));
    rows.push(run_one::<M3x2K, H06>(6, 2048, baseline));
    rows.push(run_one::<M3x4K, H06>(6, 4096, baseline));
    rows.push(run_one::<M3x8K, H06>(6, 8192, baseline));
    rows.push(run_one::<M3x2K, H07>(7, 2048, baseline));
    rows.push(run_one::<M3x4K, H07>(7, 4096, baseline));
    rows.push(run_one::<M3x8K, H07>(7, 8192, baseline));
    rows.push(run_one::<M3x2K, H08>(8, 2048, baseline));
    rows.push(run_one::<M3x4K, H08>(8, 4096, baseline));
    rows.push(run_one::<M3x8K, H08>(8, 8192, baseline));
    rows.push(run_one::<M3x2K, H09>(9, 2048, baseline));
    rows.push(run_one::<M3x4K, H09>(9, 4096, baseline));
    rows.push(run_one::<M3x8K, H09>(9, 8192, baseline));
    rows.push(run_one::<M3x2K, H10>(10, 2048, baseline));
    rows.push(run_one::<M3x4K, H10>(10, 4096, baseline));
    rows.push(run_one::<M3x8K, H10>(10, 8192, baseline));
    rows
}

fn run_one<S, H>(seed: u64, cols: usize, baseline: &BaselineData) -> AccuracyRow
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = MatrixHashType>,
{
    let _ = hash_mode_for_matrix(ROWS, cols);
    let mut sketch = CountMin::<S, FastPath, H>::from_storage(S::default());
    for &value in &baseline.values {
        sketch.insert(&SketchInput::I64(value));
    }

    let mut total_relative_error = 0.0f64;
    let mut max_relative_error = 0.0f64;
    let mut total_absolute_error = 0.0f64;
    for (value, &true_count) in &baseline.frequencies {
        let estimate_value: u64 = sketch.estimate(&SketchInput::I64(*value)).into() as u64;
        let absolute_error = estimate_value.abs_diff(true_count) as f64;
        let relative_error = absolute_error / true_count as f64;
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        if relative_error > max_relative_error {
            max_relative_error = relative_error;
        }
    }

    let distinct = baseline.distinct_items() as f64;
    AccuracyRow {
        implementation: IMPLEMENTATION_RUST_SKETCHLIB,
        language: "rust",
        seed,
        rows: ROWS,
        cols,
        total_items: baseline.total_items(),
        distinct_items: baseline.distinct_items(),
        avg_relative_error: total_relative_error / distinct,
        max_relative_error,
        mean_absolute_error: total_absolute_error / distinct,
    }
}

fn hash_input64(seed: u64, key: &SketchInput) -> u64 {
    match key {
        SketchInput::I8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::I128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::ISIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::U8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::U128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::USIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::F32(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::F64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::Str(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::String(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::Bytes(v) => XxHash3_64::oneshot_with_seed(seed, v),
    }
}

fn hash_input128(seed: u64, key: &SketchInput) -> u128 {
    match key {
        SketchInput::I8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::I128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::ISIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::U8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::U128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::USIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::F32(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::F64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::Str(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::String(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::Bytes(v) => XxHash3_128::oneshot_with_seed(seed, v),
    }
}

fn hash_heap64(seed: u64, key: &HeapItem) -> u64 {
    match key {
        HeapItem::I8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::I128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::ISIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::U8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::U128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::USIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::F32(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::F64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::String(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
    }
}

fn hash_heap128(seed: u64, key: &HeapItem) -> u128 {
    match key {
        HeapItem::I8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::I128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::ISIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::U8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::U128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::USIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::F32(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::F64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::String(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
    }
}
