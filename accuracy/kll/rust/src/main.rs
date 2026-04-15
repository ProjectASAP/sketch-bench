mod baseline;
mod output;

use baseline::{load_baseline, BaselineData};
use output::{write_csv, AccuracyRow};
use sketch_oxide::quantiles::KllSketch as OxideKll;
use asap_sketchlib::KLL;
use std::env;
use std::error::Error;
use std::path::PathBuf;

const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_kll";
const IMPLEMENTATION_RUST_OXIDE: &str = "rust_oxide_kll";
const K_LIST: &[i32] = &[50, 100, 200, 400, 800];
const NUM_PERCENTILES: usize = 101;

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
            rows.extend(run_sketchlib(&baseline));
            rows.extend(run_oxide(&baseline));
        }
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
            rows.extend(run_sketchlib(&baseline));
        }
        Some(IMPLEMENTATION_RUST_OXIDE) => {
            rows.extend(run_oxide(&baseline));
        }
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected one of \
                 {IMPLEMENTATION_RUST_SKETCHLIB}, {IMPLEMENTATION_RUST_OXIDE}"
            )
            .into());
        }
    }

    write_csv(&args.output_summary, &rows, false)?;
    Ok(())
}

fn run_sketchlib(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(K_LIST.len() * NUM_PERCENTILES);
    for &k in K_LIST {
        let mut sketch: KLL<i64> = KLL::init_kll(k);
        for &value in &baseline.values {
            sketch.update(&value);
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch.quantile(p as f64 / 100.0);
            rows.push(build_row(
                IMPLEMENTATION_RUST_SKETCHLIB,
                "rust",
                k,
                p,
                baseline.total_items(),
                true_q,
                estimate,
            ));
        }
    }
    rows
}

fn run_oxide(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(K_LIST.len() * NUM_PERCENTILES);
    for &k in K_LIST {
        let mut sketch = OxideKll::new(k as u16).expect("valid KLL k");
        for &value in &baseline.values {
            sketch.update(value as f64);
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch
                .quantile(p as f64 / 100.0)
                .expect("sketch is non-empty");
            rows.push(build_row(
                IMPLEMENTATION_RUST_OXIDE,
                "rust",
                k,
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
    k: i32,
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
        k,
        percentile,
        total_items,
        true_quantile,
        estimate,
        relative_error,
    }
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/kll_accuracy_results_rust.csv");
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
