mod baseline;
mod output;

use asap_sketchlib::{CountMin, DataInput, FastPath, NitroBatch, Vector2D};
use baseline::{BaselineData, load_baseline};
use output::{AccuracyRow, write_csv};
use std::env;
use std::error::Error;
use std::path::PathBuf;

const IMPLEMENTATION_RUST_NITRO: &str = "rust_sketchlib_nitro";
const ROWS: usize = 5;
const COLS: usize = 32768;
const NUM_TRIALS: u64 = 10;
const RATE_LIST: &[f64] = &[1.0, 0.7, 0.4, 0.1, 0.07, 0.04, 0.01];

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;
    let rows = run_nitro(&baseline)?;
    write_csv(&args.output_summary, &rows, false)?;
    Ok(())
}

fn run_nitro(baseline: &BaselineData) -> Result<Vec<AccuracyRow>, Box<dyn Error>> {
    let heavy_hitters = baseline.heavy_hitters();
    if heavy_hitters.is_empty() {
        return Err("baseline contains no heavy hitters".into());
    }

    let mut rows = Vec::with_capacity((RATE_LIST.len() as u64 * NUM_TRIALS) as usize);
    for &rate in RATE_LIST {
        for trial in 1..=NUM_TRIALS {
            let sketch = run_one(rate, baseline, &heavy_hitters);
            rows.push(AccuracyRow {
                implementation: IMPLEMENTATION_RUST_NITRO,
                language: "rust",
                trial,
                rows: ROWS,
                cols: COLS,
                rate,
                total_items: baseline.total_items(),
                distinct_items: heavy_hitters.len(),
                avg_relative_error: sketch.0,
                max_relative_error: sketch.1,
                mean_absolute_error: sketch.2,
            });
        }
    }
    Ok(rows)
}

fn run_one(rate: f64, baseline: &BaselineData, heavy_hitters: &[(i64, u64)]) -> (f64, f64, f64) {
    let target = CountMin::<Vector2D<i32>, FastPath>::with_dimensions(ROWS, COLS);
    let mut nitro = NitroBatch::with_target(rate, target);
    nitro.insert(&baseline.values);

    let mut total_relative_error = 0.0f64;
    let mut max_relative_error = 0.0f64;
    let mut total_absolute_error = 0.0f64;
    for &(value, true_count) in heavy_hitters {
        let estimate = nitro.estimate_median(&DataInput::I64(value)).max(0.0);
        let absolute_error = (estimate - true_count as f64).abs();
        let relative_error = absolute_error / true_count as f64;
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        if relative_error > max_relative_error {
            max_relative_error = relative_error;
        }
    }

    let distinct = heavy_hitters.len() as f64;
    (
        total_relative_error / distinct,
        max_relative_error,
        total_absolute_error / distinct,
    )
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/nitro_accuracy_results_rust.csv");

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
            "--help" | "-h" => {
                println!("Usage: cargo run --release -- [--data PATH] [--output-summary PATH]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output_summary,
    })
}
