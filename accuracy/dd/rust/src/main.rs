mod baseline;
mod output;

use asap_sketchlib::DDSketch;
use baseline::{BaselineData, load_baseline};
use output::{AccuracyRow, write_csv};
use std::env;
use std::error::Error;
use std::path::PathBuf;

const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_dd";
const ALPHA_LIST: &[f64] = &[0.005, 0.01, 0.02, 0.05, 0.1];
const NUM_PERCENTILES: usize = 101;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;
    let rows = run_sketchlib(&baseline);
    write_csv(&args.output_summary, &rows, false)?;
    Ok(())
}

fn run_sketchlib(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(ALPHA_LIST.len() * NUM_PERCENTILES);
    for &alpha in ALPHA_LIST {
        let mut sketch = DDSketch::new(alpha);
        for &value in &baseline.values {
            sketch.add(&(value as f64));
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch
                .get_value_at_quantile(p as f64 / 100.0)
                .expect("DDSketch is non-empty");
            rows.push(build_row(
                IMPLEMENTATION_RUST_SKETCHLIB,
                "rust",
                alpha,
                p,
                baseline.total_items(),
                true_q,
                estimate,
            ));
        }
    }
    rows
}

fn build_row(
    implementation: &'static str,
    language: &'static str,
    alpha: f64,
    percentile: usize,
    total_items: usize,
    true_quantile: f64,
    estimate: f64,
) -> AccuracyRow {
    let relative_error = if true_quantile.abs() < f64::EPSILON {
        if estimate.abs() < f64::EPSILON {
            0.0
        } else {
            estimate.abs()
        }
    } else {
        (estimate - true_quantile).abs() / true_quantile.abs()
    };

    AccuracyRow {
        implementation,
        language,
        alpha,
        percentile,
        total_items,
        true_quantile,
        estimate,
        relative_error,
    }
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/dd_accuracy_results_rust.csv");

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
