#![allow(dead_code)]

use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const DEFAULT_WARMUP_ITEMS: usize = 100_000;
pub const DEFAULT_MEASURE_ITEMS: usize = 1_000_000;
pub const DEFAULT_TRIALS: usize = 12;
pub const DEFAULT_DISCARD_TRIALS: usize = 2;

pub const HLL_PRECISION: u8 = 14;
pub const EPSILON: f64 = 0.0013;
pub const DELTA: f64 = 0.0067;
pub const ROWS: usize = 5;
pub const COLS: usize = 2048;
pub const ELASTIC_BUCKETS: usize = 1024;
pub const ELASTIC_DEPTH: usize = 3;
pub const KLL_K: i32 = 200;
pub const UNIVMON_MAX_STREAM: u64 = 256;
pub const UNIVMON_LAYERS: usize = 8;
pub const NITRO_RATE: f64 = 0.01;

pub struct BenchmarkConfig<'a> {
    pub implementation_name: &'a str,
    pub data_path: &'a Path,
    pub warmup_items: usize,
    pub measure_items: usize,
    pub trials: usize,
    pub discard_trials: usize,
}

pub struct BenchmarkResult {
    pub implementation_name: String,
    pub total_nanoseconds: u128,
}

pub fn default_data_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../input/benchmark_data_1m_int64.bin")
}

pub fn load_i64_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len() as usize;
    let num_values = file_size / std::mem::size_of::<i64>();

    let mut buffer = vec![0u8; file_size];
    file.read_exact(&mut buffer)?;

    let mut data = Vec::with_capacity(num_values);
    for chunk in buffer.chunks_exact(8) {
        let value = i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        data.push(value);
    }

    Ok(data)
}

pub fn load_string_dataset(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let values = load_i64_dataset(path)?;
    Ok(values
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<String>>())
}

pub fn load_byte_dataset(path: &Path) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let values = load_string_dataset(path)?;
    Ok(values
        .into_iter()
        .map(|v| v.into_bytes())
        .collect::<Vec<Vec<u8>>>())
}

#[inline(always)]
pub fn run_benchmark_i64<S, Build, Insert>(
    config: BenchmarkConfig<'_>,
    build_sketch: Build,
    mut insert: Insert,
) -> Result<Vec<BenchmarkResult>, Box<dyn Error>>
where
    Build: Fn() -> S + Copy,
    Insert: FnMut(&mut S, i64),
{
    let data = std::hint::black_box(load_i64_dataset(config.data_path)?);
    let measure_count = config.measure_items.min(data.len());

    let mut results = Vec::with_capacity(config.trials - config.discard_trials);

    for trial in 0..config.trials {
        let mut sketch = build_sketch();
        let start = Instant::now();
        for &value in data.iter().take(measure_count) {
            insert(&mut sketch, value);
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();

        if trial >= config.discard_trials {
            results.push(BenchmarkResult {
                implementation_name: config.implementation_name.to_string(),
                total_nanoseconds: elapsed,
            });
        }
    }

    Ok(results)
}

pub fn run_benchmark_i64_batch<S, Build, Insert>(
    config: BenchmarkConfig<'_>,
    build_sketch: Build,
    mut insert_batch: Insert,
) -> Result<Vec<BenchmarkResult>, Box<dyn Error>>
where
    Build: Fn() -> S + Copy,
    Insert: FnMut(&mut S, &[i64]),
{
    let data = load_i64_dataset(config.data_path)?;
    let measure_count = config.measure_items.min(data.len());

    let mut results = Vec::with_capacity(config.trials - config.discard_trials);

    for trial in 0..config.trials {
        let mut sketch = build_sketch();
        let start = Instant::now();
        insert_batch(&mut sketch, &data[..measure_count]);
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();

        if trial >= config.discard_trials {
            results.push(BenchmarkResult {
                implementation_name: config.implementation_name.to_string(),
                total_nanoseconds: elapsed,
            });
        }
    }

    Ok(results)
}

pub fn run_benchmark_bytes<S, Build, Insert>(
    config: BenchmarkConfig<'_>,
    build_sketch: Build,
    mut insert: Insert,
) -> Result<Vec<BenchmarkResult>, Box<dyn Error>>
where
    Build: Fn() -> S + Copy,
    Insert: FnMut(&mut S, &[u8]),
{
    let data = std::hint::black_box(load_byte_dataset(config.data_path)?);
    let measure_count = config.measure_items.min(data.len());

    let mut results = Vec::with_capacity(config.trials - config.discard_trials);

    for trial in 0..config.trials {
        let mut sketch = build_sketch();
        let start = Instant::now();
        for key in data.iter().take(measure_count) {
            insert(&mut sketch, key.as_slice());
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();

        if trial >= config.discard_trials {
            results.push(BenchmarkResult {
                implementation_name: config.implementation_name.to_string(),
                total_nanoseconds: elapsed,
            });
        }
    }

    Ok(results)
}

pub fn run_benchmark_strings<S, Build, Insert>(
    config: BenchmarkConfig<'_>,
    build_sketch: Build,
    mut insert: Insert,
) -> Result<Vec<BenchmarkResult>, Box<dyn Error>>
where
    Build: Fn() -> S + Copy,
    Insert: FnMut(&mut S, &str),
{
    let data = std::hint::black_box(load_string_dataset(config.data_path)?);
    let measure_count = config.measure_items.min(data.len());

    let mut results = Vec::with_capacity(config.trials - config.discard_trials);

    for trial in 0..config.trials {
        let mut sketch = build_sketch();
        let start = Instant::now();
        for key in data.iter().take(measure_count) {
            insert(&mut sketch, key.as_str());
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();

        if trial >= config.discard_trials {
            results.push(BenchmarkResult {
                implementation_name: config.implementation_name.to_string(),
                total_nanoseconds: elapsed,
            });
        }
    }

    Ok(results)
}

pub fn print_result(result: &BenchmarkResult) {
    println!(
        "{{\"implementation_name\":\"{}\",\"total_nanoseconds\":{}}}",
        result.implementation_name, result.total_nanoseconds
    );
}

pub fn print_results(results: &[BenchmarkResult]) {
    for result in results {
        print_result(result);
    }
}
