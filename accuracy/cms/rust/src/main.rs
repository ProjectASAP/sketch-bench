mod baseline;
mod config;
mod datasketches_runner;
mod output;
mod oxide_runner;
mod seeds;
mod sketchlib_runner;

use baseline::load_baseline;
use config::{
    IMPLEMENTATION_RUST_DATASKETCHES, IMPLEMENTATION_RUST_OXIDE, IMPLEMENTATION_RUST_SKETCHLIB,
};
use output::{write_csv, KeyErrorCsvWriter, KeySeedErrorCsvWriter};
use std::env;
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
    output_key_errors: PathBuf,
    output_key_seed_errors: PathBuf,
    skip_summary: bool,
    skip_key_errors: bool,
    skip_key_seed_errors: bool,
    seed_filter: Option<u64>,
    cols_filter: Option<usize>,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    if args.skip_summary && args.skip_key_errors && args.skip_key_seed_errors {
        return Err("cannot skip summary, key-error, and key-seed-error outputs together".into());
    }

    if !args.skip_summary {
        let mut rows = Vec::new();
        match args.implementation_filter.as_deref() {
            None => {
                rows.extend(datasketches_runner::run_summary(&baseline));
                rows.extend(oxide_runner::run_summary(&baseline));
                rows.extend(sketchlib_runner::run_summary(&baseline));
            }
            Some(IMPLEMENTATION_RUST_DATASKETCHES) => {
                rows.extend(datasketches_runner::run_summary(&baseline))
            }
            Some(IMPLEMENTATION_RUST_OXIDE) => rows.extend(oxide_runner::run_summary(&baseline)),
            Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
                rows.extend(sketchlib_runner::run_summary(&baseline))
            }
            Some(other) => {
                return Err(format!(
                    "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_DATASKETCHES}, {IMPLEMENTATION_RUST_OXIDE}, or {IMPLEMENTATION_RUST_SKETCHLIB}"
                )
                .into())
            }
        }
        write_csv(&args.output_summary, &rows, false)?;
    }

    if !args.skip_key_errors {
        let mut writer = KeyErrorCsvWriter::create(&args.output_key_errors, false)?;
        match args.implementation_filter.as_deref() {
            None => {
                datasketches_runner::write_key_median_errors(&baseline, &mut writer)?;
                oxide_runner::write_key_median_errors(&baseline, &mut writer)?;
                sketchlib_runner::write_key_median_errors(&baseline, &mut writer)?;
            }
            Some(IMPLEMENTATION_RUST_DATASKETCHES) => {
                datasketches_runner::write_key_median_errors(&baseline, &mut writer)?
            }
            Some(IMPLEMENTATION_RUST_OXIDE) => {
                oxide_runner::write_key_median_errors(&baseline, &mut writer)?
            }
            Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
                sketchlib_runner::write_key_median_errors(&baseline, &mut writer)?
            }
            Some(other) => {
                return Err(format!(
                    "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_DATASKETCHES}, {IMPLEMENTATION_RUST_OXIDE}, or {IMPLEMENTATION_RUST_SKETCHLIB}"
                )
                .into())
            }
        }
        writer.flush()?;
    }

    if !args.skip_key_seed_errors {
        let mut writer = KeySeedErrorCsvWriter::create(&args.output_key_seed_errors, false)?;
        match args.implementation_filter.as_deref() {
            None => {
                datasketches_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?;
                oxide_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?;
                sketchlib_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?;
            }
            Some(IMPLEMENTATION_RUST_DATASKETCHES) => {
                datasketches_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?
            }
            Some(IMPLEMENTATION_RUST_OXIDE) => {
                oxide_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?
            }
            Some(IMPLEMENTATION_RUST_SKETCHLIB) => {
                sketchlib_runner::write_key_seed_errors(
                    &baseline,
                    &mut writer,
                    args.seed_filter,
                    args.cols_filter,
                )?
            }
            Some(other) => {
                return Err(format!(
                    "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_DATASKETCHES}, {IMPLEMENTATION_RUST_OXIDE}, or {IMPLEMENTATION_RUST_SKETCHLIB}"
                )
                .into())
            }
        }
        writer.flush()?;
    }

    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/cms_accuracy_results.csv");
    let mut output_key_errors = PathBuf::from("../output/cms_accuracy_key_median_errors.csv");
    let mut output_key_seed_errors = PathBuf::from("../output/cms_accuracy_key_seed_errors.csv");
    let mut skip_summary = false;
    let mut skip_key_errors = false;
    let mut skip_key_seed_errors = false;
    let mut seed_filter = None;
    let mut cols_filter = None;
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
            "--output-key-seed-errors" => {
                output_key_seed_errors = PathBuf::from(
                    args.next()
                        .ok_or("--output-key-seed-errors requires a path")?,
                );
            }
            "--skip-summary" => skip_summary = true,
            "--skip-key-errors" => skip_key_errors = true,
            "--skip-key-seed-errors" => skip_key_seed_errors = true,
            "--seed" => {
                seed_filter = Some(args.next().ok_or("--seed requires a value")?.parse()?);
            }
            "--cols" => {
                cols_filter = Some(args.next().ok_or("--cols requires a value")?.parse()?);
            }
            "--impl" => {
                implementation_filter = Some(args.next().ok_or("--impl requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release -- [--data PATH] [--output-summary PATH] [--output-key-errors PATH] [--output-key-seed-errors PATH] [--skip-summary] [--skip-key-errors] [--skip-key-seed-errors] [--seed N] [--cols N] [--impl NAME]"
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
        output_key_seed_errors,
        skip_summary,
        skip_key_errors,
        skip_key_seed_errors,
        seed_filter,
        cols_filter,
        implementation_filter,
    })
}
