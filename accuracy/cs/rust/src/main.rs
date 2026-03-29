mod baseline;
mod config;
mod output;
mod seeds;
mod sketchlib_runner;

use baseline::load_baseline;
use config::IMPLEMENTATION_RUST_SKETCHLIB;
use output::{write_csv, KeyErrorCsvWriter};
use std::env;
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
    output_key_errors: PathBuf,
    skip_summary: bool,
    skip_key_errors: bool,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    if args.skip_summary && args.skip_key_errors {
        return Err("cannot skip both summary and key-error outputs".into());
    }

    if !args.skip_summary {
        let mut rows = Vec::new();
        match args.implementation_filter.as_deref() {
            None | Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
                rows.extend(sketchlib_runner::run_summary(&baseline))
            }
            Some(other) => {
                return Err(format!(
                    "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_SKETCHLIB}"
                )
                .into())
            }
        }
        write_csv(&args.output_summary, &rows, false)?;
    }

    if !args.skip_key_errors {
        let mut writer = KeyErrorCsvWriter::create(&args.output_key_errors, false)?;
        match args.implementation_filter.as_deref() {
            None | Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
                sketchlib_runner::write_key_median_errors(&baseline, &mut writer)?
            }
            Some(other) => {
                return Err(format!(
                    "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_SKETCHLIB}"
                )
                .into())
            }
        }
        writer.flush()?;
    }

    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../data/benchmark_data_1m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/cms_accuracy_results.csv");
    let mut output_key_errors = PathBuf::from("../output/cms_accuracy_key_median_errors.csv");
    let mut skip_summary = false;
    let mut skip_key_errors = false;
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
            "--output-key-errors" => {
                output_key_errors =
                    PathBuf::from(args.next().ok_or("--output-key-errors requires a path")?);
            }
            "--skip-summary" => skip_summary = true,
            "--skip-key-errors" => skip_key_errors = true,
            "--impl" => {
                implementation_filter = Some(args.next().ok_or("--impl requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release -- [--data PATH] [--output-summary PATH] [--output-key-errors PATH] [--skip-summary] [--skip-key-errors] [--impl NAME]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output_summary,
        output_key_errors,
        skip_summary,
        skip_key_errors,
        implementation_filter,
    })
}
