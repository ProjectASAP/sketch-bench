use crate::baseline::BaselineData;
use crate::config::{COLS_LIST, IMPLEMENTATION_RUST_SKETCHLIB, ROWS};
use crate::output::{AccuracyRow, KeyErrorCsvWriter, KeyMedianErrorRow};
use sketchlib_rust::{
    impl_fixed_matrix, CountMin, FastPath, HeapItem, MatrixStorage, SketchHasher, SketchInput,
};
use std::io;
use twox_hash::{XxHash3_128, XxHash3_64};

impl_fixed_matrix!(M5x2K, i32, 5, 2048);
impl_fixed_matrix!(M5x4K, i32, 5, 4096);
impl_fixed_matrix!(M5x8K, i32, 5, 8192);
impl_fixed_matrix!(M5x16K, i32, 5, 16384);
impl_fixed_matrix!(M5x32K, i32, 5, 32768);
impl_fixed_matrix!(M5x64K, i32, 5, 65536);
impl_fixed_matrix!(M5x128K, i32, 5, 131072);

macro_rules! define_seeded_hasher {
    ($name:ident, $seed:expr) => {
        #[derive(Clone, Debug)]
        struct $name;

        impl SketchHasher for $name {
            type HashType = u128;

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
                cols: usize,
                key: &SketchInput,
            ) -> Self::HashType {
                let mask_bits = if cols.is_power_of_two() {
                    cols.ilog2() as usize
                } else {
                    cols.ilog2() as usize + 1
                };
                let col_mask = (1u128 << mask_bits) - 1;
                let mut packed = 0u128;
                for row in 0..rows {
                    let row_hash = Self::hash128_seeded(row, key);
                    let col_bits = row_hash & col_mask;
                    packed |= col_bits << (mask_bits * row);
                    let sign_bit = (row_hash >> 127) & 1;
                    packed |= sign_bit << (127 - row);
                }
                packed
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

pub fn run_summary(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(10 * COLS_LIST.len());
    macro_rules! push_runs {
        ($seed:expr, $hasher:ty) => {
            rows.push(run_one::<M5x2K, $hasher>($seed, 2048, baseline));
            rows.push(run_one::<M5x4K, $hasher>($seed, 4096, baseline));
            rows.push(run_one::<M5x8K, $hasher>($seed, 8192, baseline));
            rows.push(run_one::<M5x16K, $hasher>($seed, 16384, baseline));
            rows.push(run_one::<M5x32K, $hasher>($seed, 32768, baseline));
            rows.push(run_one::<M5x64K, $hasher>($seed, 65536, baseline));
            rows.push(run_one::<M5x128K, $hasher>($seed, 131072, baseline));
        };
    }
    push_runs!(1, H01);
    push_runs!(2, H02);
    push_runs!(3, H03);
    push_runs!(4, H04);
    push_runs!(5, H05);
    push_runs!(6, H06);
    push_runs!(7, H07);
    push_runs!(8, H08);
    push_runs!(9, H09);
    push_runs!(10, H10);
    rows
}

pub fn write_key_median_errors(
    baseline: &BaselineData,
    writer: &mut KeyErrorCsvWriter,
) -> io::Result<()> {
    write_group_medians_m5x2k(baseline, writer)?;
    write_group_medians_m5x4k(baseline, writer)?;
    write_group_medians_m5x8k(baseline, writer)?;
    write_group_medians_m5x16k(baseline, writer)?;
    write_group_medians_m5x32k(baseline, writer)?;
    write_group_medians_m5x64k(baseline, writer)?;
    write_group_medians_m5x128k(baseline, writer)?;
    Ok(())
}

fn run_one<S, H>(seed: u64, cols: usize, baseline: &BaselineData) -> AccuracyRow
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = u128>,
{
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

fn median_estimator<S, H>(baseline: &BaselineData) -> CountMin<S, FastPath, H>
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = u128>,
{
    let mut sketch = CountMin::<S, FastPath, H>::from_storage(S::default());
    for &value in &baseline.values {
        sketch.insert(&SketchInput::I64(value));
    }
    sketch
}

macro_rules! write_group_medians {
    ($fn_name:ident, $storage:ty, $cols:expr) => {
        fn $fn_name(baseline: &BaselineData, writer: &mut KeyErrorCsvWriter) -> io::Result<()> {
            let s01 = median_estimator::<$storage, H01>(baseline);
            let s02 = median_estimator::<$storage, H02>(baseline);
            let s03 = median_estimator::<$storage, H03>(baseline);
            let s04 = median_estimator::<$storage, H04>(baseline);
            let s05 = median_estimator::<$storage, H05>(baseline);
            let s06 = median_estimator::<$storage, H06>(baseline);
            let s07 = median_estimator::<$storage, H07>(baseline);
            let s08 = median_estimator::<$storage, H08>(baseline);
            let s09 = median_estimator::<$storage, H09>(baseline);
            let s10 = median_estimator::<$storage, H10>(baseline);

            for (&key, &true_count) in &baseline.frequencies {
                let value = SketchInput::I64(key);
                let mut estimates = [
                    {
                        let estimate: i64 = s01.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s02.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s03.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s04.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s05.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s06.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s07.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s08.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s09.estimate(&value).into();
                        estimate as u64
                    },
                    {
                        let estimate: i64 = s10.estimate(&value).into();
                        estimate as u64
                    },
                ];
                estimates.sort_unstable();
                let median_estimate = estimates[(estimates.len() / 2) - 1];
                let median_relative_error =
                    median_estimate.abs_diff(true_count) as f64 / true_count as f64;
                writer.write_row(&KeyMedianErrorRow {
                    implementation: IMPLEMENTATION_RUST_SKETCHLIB,
                    language: "rust",
                    rows: ROWS,
                    cols: $cols,
                    key,
                    true_count,
                    median_estimate,
                    median_relative_error,
                })?;
            }
            Ok(())
        }
    };
}

write_group_medians!(write_group_medians_m5x2k, M5x2K, 2048);
write_group_medians!(write_group_medians_m5x4k, M5x4K, 4096);
write_group_medians!(write_group_medians_m5x8k, M5x8K, 8192);
write_group_medians!(write_group_medians_m5x16k, M5x16K, 16384);
write_group_medians!(write_group_medians_m5x32k, M5x32K, 32768);
write_group_medians!(write_group_medians_m5x64k, M5x64K, 65536);
write_group_medians!(write_group_medians_m5x128k, M5x128K, 131072);

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
