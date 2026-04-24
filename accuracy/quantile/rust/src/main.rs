//! Unified accuracy harness for the **quantile** statistic.
//!
//! KLL and DDSketch both answer rank/quantile queries against a
//! single sorted stream, so they share this binary rather than
//! getting their own accuracy crates. Family-specific runners +
//! CSV schemas live under `kll/` and `dd/`; the ground-truth
//! baseline lives at the crate root and delegates to
//! `sketch_bench::baselines::ExactQuantile`.
//!
//! Usage:
//!
//!     quantile_accuracy --sketch kll|dd \
//!         --data <path> --output-summary <path> \
//!         [--impl NAME]

mod baseline;
mod dd;
mod kll;

use std::env;
use std::error::Error;
use std::path::PathBuf;

use baseline::load_baseline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sketch {
    Kll,
    Dd,
}

impl Sketch {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "kll" => Ok(Self::Kll),
            "dd" | "ddsketch" => Ok(Self::Dd),
            other => Err(format!(
                "unknown --sketch value: {other}; expected `kll` or `dd`"
            )),
        }
    }
}

#[derive(Debug)]
struct Args {
    sketch: Sketch,
    data: PathBuf,
    output_summary: PathBuf,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    let impl_filter = args.implementation_filter.as_deref();
    match args.sketch {
        Sketch::Kll => {
            let mut rows = Vec::new();
            if matches(impl_filter, kll::runner::IMPLEMENTATION_RUST_SKETCHLIB) {
                rows.extend(kll::runner::run_sketchlib(&baseline));
            }
            if matches(impl_filter, kll::runner::IMPLEMENTATION_RUST_OXIDE) {
                rows.extend(kll::runner::run_oxide(&baseline));
            }
            if rows.is_empty() {
                return Err(format!("no KLL runners matched --impl={impl_filter:?}").into());
            }
            kll::output::write_csv(&args.output_summary, &rows, false)?;
        }
        Sketch::Dd => {
            let mut rows = Vec::new();
            if matches(impl_filter, dd::runner::IMPLEMENTATION_RUST_SKETCHLIB) {
                rows.extend(dd::runner::run_sketchlib(&baseline));
            }
            if rows.is_empty() {
                return Err(format!("no DDSketch runners matched --impl={impl_filter:?}").into());
            }
            dd::output::write_csv(&args.output_summary, &rows, false)?;
        }
    }

    Ok(())
}

fn matches(filter: Option<&str>, candidate: &str) -> bool {
    match filter {
        None => true,
        Some(f) => f == candidate,
    }
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut sketch = Sketch::Kll;
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/quantile_accuracy_results.csv");
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sketch" => {
                let v = args.next().ok_or("--sketch requires a value")?;
                sketch = Sketch::parse(&v)?;
            }
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
                    "Usage: quantile_accuracy --sketch kll|dd [--data PATH] \
                     [--output-summary PATH] [--impl NAME]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        sketch,
        data,
        output_summary,
        implementation_filter,
    })
}
