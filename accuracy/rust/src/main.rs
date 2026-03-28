mod baseline;
mod config;
mod datasketches_runner;
mod output;
mod seeds;
mod sketchlib_runner;

use baseline::load_baseline;
use config::{IMPLEMENTATION_RUST_DATASKETCHES, IMPLEMENTATION_RUST_SKETCHLIB};
use output::write_csv;
use std::env;
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output: PathBuf,
    append: bool,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    let mut rows = Vec::new();
    match args.implementation_filter.as_deref() {
        None => {
            rows.extend(datasketches_runner::run(&baseline));
            rows.extend(sketchlib_runner::run(&baseline));
        }
        Some(IMPLEMENTATION_RUST_DATASKETCHES) => rows.extend(datasketches_runner::run(&baseline)),
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => rows.extend(sketchlib_runner::run(&baseline)),
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected {IMPLEMENTATION_RUST_DATASKETCHES} or {IMPLEMENTATION_RUST_SKETCHLIB}"
            )
            .into())
        }
    }

    write_csv(&args.output, &rows, args.append)?;
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../data/benchmark_data_1m_int64.bin");
    let mut output = PathBuf::from("../output/cms_accuracy_results.csv");
    let mut append = false;
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                data = PathBuf::from(args.next().ok_or("--data requires a path")?);
            }
            "--output" => {
                output = PathBuf::from(args.next().ok_or("--output requires a path")?);
            }
            "--append" => append = true,
            "--impl" => {
                implementation_filter = Some(args.next().ok_or("--impl requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release -- [--data PATH] [--output PATH] [--append] [--impl NAME]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output,
        append,
        implementation_filter,
    })
}
