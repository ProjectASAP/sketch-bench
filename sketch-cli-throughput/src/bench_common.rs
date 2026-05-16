//! Phase 1 probe of the sketchlib-vs-throughput-bench gap.
//!
//! Literal copy of `throughput-bench/src/bench_common.rs` so the
//! only variable between this binary and that one is the
//! workspace it's compiled in (different `[profile.release]`,
//! different default crate-graph). If this binary clocks the
//! same ~9 ms/trial as throughput-bench on the same hot loop,
//! the gap vs sketchlib is binary-composition driven (link-unit
//! size pushing LLVM's specialization decisions on
//! `asap_sketchlib::Count::insert`).

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
    // sketch-cli-throughput sits at the workspace root next to
    // throughput-bench/, so the same ../input relative path works.
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

fn warmup_cpu(secs: u64) {
    let deadline = Instant::now() + std::time::Duration::from_secs(secs);
    let mut x: u64 = 0xdeadbeef;
    while Instant::now() < deadline {
        for _ in 0..10_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        std::hint::black_box(x);
    }
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

    let warmup_secs: u64 = std::env::var("BENCH_WARMUP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    if warmup_secs > 0 {
        warmup_cpu(warmup_secs);
    }

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

pub fn print_results(results: &[BenchmarkResult]) {
    for r in results {
        println!(
            "{{\"implementation_name\":\"{}\",\"total_nanoseconds\":{}}}",
            r.implementation_name, r.total_nanoseconds
        );
    }
}
