mod baseline;
mod output;
mod seeds;

use baseline::load_baseline;
use datasketches::hll::{HllSketch, HllType};
use output::{write_csv, AccuracyRow};
use seeds::SEEDS;
use sketchlib_rust::{DataFusion, HyperLogLog, SketchInput};
use std::env;
use std::error::Error;
use std::path::PathBuf;

const IMPLEMENTATION_RUST_DATASKETCHES: &str = "rust_datasketches_hll";
const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_hll";
const LG_K: u8 = 14;
const REGISTERS: usize = 1usize << LG_K;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    let mut rows = Vec::new();
    match args.implementation_filter.as_deref() {
        None => {
            rows.extend(run_sketchlib(&baseline.values, baseline.distinct_items()));
            rows.extend(run_datasketches_rust(
                &baseline.values,
                baseline.distinct_items(),
            ));
        }
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
            rows.extend(run_sketchlib(&baseline.values, baseline.distinct_items()));
        }
        Some(IMPLEMENTATION_RUST_DATASKETCHES) => {
            rows.extend(run_datasketches_rust(
                &baseline.values,
                baseline.distinct_items(),
            ));
        }
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected one of {IMPLEMENTATION_RUST_SKETCHLIB}, {IMPLEMENTATION_RUST_DATASKETCHES}"
            )
            .into());
        }
    }

    write_csv(&args.output_summary, &rows, false)?;
    Ok(())
}

fn run_sketchlib(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let mut sketch = HyperLogLog::<DataFusion>::new();
        for &value in values {
            sketch.insert(&SketchInput::U64(seeded_key(value, seed)));
        }
        rows.push(build_row(
            IMPLEMENTATION_RUST_SKETCHLIB,
            "rust",
            seed,
            values.len(),
            true_distinct,
            sketch.estimate() as f64,
        ));
    }
    rows
}

fn run_datasketches_rust(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let mut sketch = HllSketch::new(LG_K, HllType::Hll8);
        for &value in values {
            sketch.update(seeded_key(value, seed));
        }
        rows.push(build_row(
            IMPLEMENTATION_RUST_DATASKETCHES,
            "rust",
            seed,
            values.len(),
            true_distinct,
            sketch.estimate(),
        ));
    }
    rows
}

fn build_row(
    implementation: &'static str,
    language: &'static str,
    seed: u64,
    total_items: usize,
    true_distinct: usize,
    estimate: f64,
) -> AccuracyRow {
    let relative_error = (estimate - true_distinct as f64).abs() / true_distinct as f64;
    AccuracyRow {
        implementation,
        language,
        seed,
        lg_k: LG_K,
        registers: REGISTERS,
        total_items,
        true_distinct,
        estimate,
        relative_error,
    }
}

fn seeded_key(value: i64, seed: u64) -> u64 {
    splitmix64((value as u64) ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/hll_accuracy_results_rust.csv");
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                data = PathBuf::from(args.next().ok_or("--data requires a path")?);
            }
            "--output-summary" => {
                output_summary =
                    PathBuf::from(args.next().ok_or("--output-summary requires a path")?);
            }
            "--impl" => {
                implementation_filter = Some(args.next().ok_or("--impl requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release -- [--data PATH] [--output-summary PATH] [--impl NAME]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output_summary,
        implementation_filter,
    })
}
