use crate::baseline::BaselineData;
use crate::config::{COLS_LIST, IMPLEMENTATION_RUST_SKETCHLIB, ROWS};
use crate::output::{
    AccuracyRow, KeyErrorCsvWriter, KeyMedianErrorRow, KeySeedErrorCsvWriter, KeySeedErrorRow,
};
use sketchlib_rust::{
    impl_fixed_matrix, hash_for_matrix_seeded_generic, CountMin, FastPath, HeapItem,
    MatrixHashType, MatrixStorage, SketchHasher, SketchInput,
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
                seed_idx: usize,
                rows: usize,
                cols: usize,
                key: &SketchInput,
            ) -> Self::HashType {
                hash_for_matrix_seeded_generic::<Self>(seed_idx, rows, cols, key)
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
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    let mut rows = Vec::with_capacity(10 * COLS_LIST.len());
    macro_rules! push_runs {
        ($seed:expr, $hasher:ty) => {
            rows.push(run_one::<M5x2K, $hasher>($seed, 2048, baseline, &heavy_hitters));
            rows.push(run_one::<M5x4K, $hasher>($seed, 4096, baseline, &heavy_hitters));
            rows.push(run_one::<M5x8K, $hasher>($seed, 8192, baseline, &heavy_hitters));
            rows.push(run_one::<M5x16K, $hasher>($seed, 16384, baseline, &heavy_hitters));
            rows.push(run_one::<M5x32K, $hasher>($seed, 32768, baseline, &heavy_hitters));
            rows.push(run_one::<M5x64K, $hasher>($seed, 65536, baseline, &heavy_hitters));
            rows.push(run_one::<M5x128K, $hasher>($seed, 131072, baseline, &heavy_hitters));
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
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    write_group_medians_m5x2k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x4k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x8k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x16k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x32k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x64k(baseline, &heavy_hitters, writer)?;
    write_group_medians_m5x128k(baseline, &heavy_hitters, writer)?;
    Ok(())
}

pub fn write_key_seed_errors(
    baseline: &BaselineData,
    writer: &mut KeySeedErrorCsvWriter,
    seed_filter: Option<u64>,
    cols_filter: Option<usize>,
) -> io::Result<()> {
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    maybe_write_seed_errors_for_group::<M5x2K, H01>(1, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H01>(1, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H01>(1, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H01>(1, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H01>(1, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H01>(1, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H01>(1, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H02>(2, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H02>(2, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H02>(2, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H02>(2, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H02>(2, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H02>(2, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H02>(2, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H03>(3, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H03>(3, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H03>(3, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H03>(3, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H03>(3, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H03>(3, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H03>(3, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H04>(4, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H04>(4, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H04>(4, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H04>(4, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H04>(4, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H04>(4, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H04>(4, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H05>(5, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H05>(5, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H05>(5, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H05>(5, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H05>(5, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H05>(5, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H05>(5, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H06>(6, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H06>(6, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H06>(6, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H06>(6, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H06>(6, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H06>(6, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H06>(6, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H07>(7, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H07>(7, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H07>(7, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H07>(7, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H07>(7, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H07>(7, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H07>(7, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H08>(8, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H08>(8, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H08>(8, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H08>(8, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H08>(8, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H08>(8, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H08>(8, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H09>(9, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H09>(9, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H09>(9, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H09>(9, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H09>(9, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H09>(9, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H09>(9, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;

    maybe_write_seed_errors_for_group::<M5x2K, H10>(10, 2048, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x4K, H10>(10, 4096, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x8K, H10>(10, 8192, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x16K, H10>(10, 16384, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x32K, H10>(10, 32768, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x64K, H10>(10, 65536, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    maybe_write_seed_errors_for_group::<M5x128K, H10>(10, 131072, baseline, &heavy_hitters, writer, seed_filter, cols_filter)?;
    Ok(())
}

fn run_one<S, H>(
    seed: u64,
    cols: usize,
    baseline: &BaselineData,
    heavy_hitters: &[(i64, u64)],
) -> AccuracyRow
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = MatrixHashType>,
{
    let mut sketch = CountMin::<S, FastPath, H>::from_storage(S::default());
    for &value in &baseline.values {
        sketch.insert(&SketchInput::I64(value));
    }

    let mut total_relative_error = 0.0f64;
    let mut max_relative_error = 0.0f64;
    let mut total_absolute_error = 0.0f64;
    for &(value, true_count) in heavy_hitters {
        let estimate_value: u64 = sketch.estimate(&SketchInput::I64(value)).into() as u64;
        let absolute_error = estimate_value.abs_diff(true_count) as f64;
        let relative_error = absolute_error / true_count as f64;
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        if relative_error > max_relative_error {
            max_relative_error = relative_error;
        }
    }

    let distinct = heavy_hitters.len() as f64;
    AccuracyRow {
        implementation: IMPLEMENTATION_RUST_SKETCHLIB,
        language: "rust",
        seed,
        rows: ROWS,
        cols,
        total_items: baseline.total_items(),
        distinct_items: heavy_hitters.len(),
        avg_relative_error: total_relative_error / distinct,
        max_relative_error,
        mean_absolute_error: total_absolute_error / distinct,
    }
}

fn median_estimator<S, H>(baseline: &BaselineData) -> CountMin<S, FastPath, H>
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = MatrixHashType>,
{
    let mut sketch = CountMin::<S, FastPath, H>::from_storage(S::default());
    for &value in &baseline.values {
        sketch.insert(&SketchInput::I64(value));
    }
    sketch
}

fn write_seed_errors_for_group<S, H>(
    seed: u64,
    cols: usize,
    baseline: &BaselineData,
    heavy_hitters: &[(i64, u64)],
    writer: &mut KeySeedErrorCsvWriter,
) -> io::Result<()>
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = MatrixHashType>,
{
    let sketch = median_estimator::<S, H>(baseline);
    for &(key, true_count) in heavy_hitters {
        let estimate: u64 = sketch.estimate(&SketchInput::I64(key)).into() as u64;
        let relative_error = estimate.abs_diff(true_count) as f64 / true_count as f64;
        writer.write_row(&KeySeedErrorRow {
            implementation: IMPLEMENTATION_RUST_SKETCHLIB,
            language: "rust",
            seed,
            rows: ROWS,
            cols,
            key,
            true_count,
            estimate,
            relative_error,
        })?;
    }
    Ok(())
}

fn maybe_write_seed_errors_for_group<S, H>(
    seed: u64,
    cols: usize,
    baseline: &BaselineData,
    heavy_hitters: &[(i64, u64)],
    writer: &mut KeySeedErrorCsvWriter,
    seed_filter: Option<u64>,
    cols_filter: Option<usize>,
) -> io::Result<()>
where
    S: MatrixStorage + Default + sketchlib_rust::FastPathHasher<H>,
    S::Counter: Copy + PartialOrd + From<i32> + std::ops::AddAssign + Into<i64>,
    H: SketchHasher<HashType = MatrixHashType>,
{
    if seed_filter.is_some_and(|required| required != seed) {
        return Ok(());
    }
    if cols_filter.is_some_and(|required| required != cols) {
        return Ok(());
    }
    write_seed_errors_for_group::<S, H>(seed, cols, baseline, heavy_hitters, writer)
}

macro_rules! write_group_medians {
    ($fn_name:ident, $storage:ty, $cols:expr) => {
        fn $fn_name(
            baseline: &BaselineData,
            heavy_hitters: &[(i64, u64)],
            writer: &mut KeyErrorCsvWriter,
        ) -> io::Result<()> {
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

            for &(key, true_count) in heavy_hitters {
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

fn heavy_hitters_or_panic(baseline: &BaselineData) -> Vec<(i64, u64)> {
    let heavy_hitters = baseline.heavy_hitters();
    assert!(
        !heavy_hitters.is_empty(),
        "baseline contains no heavy hitters with true_count >= 100"
    );
    heavy_hitters
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
