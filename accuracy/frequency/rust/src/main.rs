//! Unified accuracy harness for the **frequency** statistic.
//!
//! CMS and Count-Sketch both answer per-key frequency queries and
//! derive heavy-hitter sets from them, so they share this binary
//! rather than getting their own accuracy crate. Runners live
//! under `cms/*` and `countsketch/*`; everything else (baseline
//! loader, CSV row schemas, seeds, sweep config) is shared at the
//! crate root.
//!
//! Usage:
//!
//!     frequency_accuracy --sketch cms|countsketch \
//!         --data <path> \
//!         --output-summary <path> \
//!         [--output-key-errors <path>] \
//!         [--output-key-seed-errors <path>] \
//!         [--impl NAME] [--seed N] [--cols N]

mod baseline;
mod cms;
mod config;
mod countsketch;
mod output;
mod seeds;

use std::env;
use std::error::Error;
use std::path::PathBuf;

use baseline::load_baseline;
use output::{write_csv, KeyErrorCsvWriter, KeySeedErrorCsvWriter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sketch {
    Cms,
    CountSketch,
}

impl Sketch {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "cms" | "count-min" | "countmin" => Ok(Self::Cms),
            "cs" | "count" | "countsketch" | "count-sketch" => Ok(Self::CountSketch),
            other => Err(format!(
                "unknown --sketch value: {other}; expected `cms` or `countsketch`"
            )),
        }
    }
}

#[derive(Debug)]
struct Args {
    sketch: Sketch,
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

    let impl_filter = args.implementation_filter.as_deref();

    if !args.skip_summary {
        let mut rows = Vec::new();
        match args.sketch {
            Sketch::Cms => {
                if matches(impl_filter, config::cms_impl::DATASKETCHES) {
                    rows.extend(cms::datasketches::run_summary(&baseline));
                }
                if matches(impl_filter, config::cms_impl::OXIDE) {
                    rows.extend(cms::oxide::run_summary(&baseline));
                }
                if matches(impl_filter, config::cms_impl::SKETCHLIB) {
                    rows.extend(cms::sketchlib::run_summary(&baseline));
                }
            }
            Sketch::CountSketch => {
                if matches(impl_filter, config::cs_impl::OXIDE) {
                    rows.extend(countsketch::oxide::run_summary(&baseline));
                }
                if matches(impl_filter, config::cs_impl::SKETCHLIB) {
                    rows.extend(countsketch::sketchlib::run_summary(&baseline));
                }
            }
        }
        if rows.is_empty() {
            return Err(format!(
                "no runners matched --impl={impl_filter:?} for --sketch={:?}",
                args.sketch,
            )
            .into());
        }
        write_csv(&args.output_summary, &rows, false)?;
    }

    if !args.skip_key_errors {
        let mut writer = KeyErrorCsvWriter::create(&args.output_key_errors, false)?;
        match args.sketch {
            Sketch::Cms => {
                if matches(impl_filter, config::cms_impl::DATASKETCHES) {
                    cms::datasketches::write_key_median_errors(&baseline, &mut writer)?;
                }
                if matches(impl_filter, config::cms_impl::OXIDE) {
                    cms::oxide::write_key_median_errors(&baseline, &mut writer)?;
                }
                if matches(impl_filter, config::cms_impl::SKETCHLIB) {
                    cms::sketchlib::write_key_median_errors(&baseline, &mut writer)?;
                }
            }
            Sketch::CountSketch => {
                if matches(impl_filter, config::cs_impl::OXIDE) {
                    countsketch::oxide::write_key_median_errors(&baseline, &mut writer)?;
                }
                if matches(impl_filter, config::cs_impl::SKETCHLIB) {
                    countsketch::sketchlib::write_key_median_errors(&baseline, &mut writer)?;
                }
            }
        }
        writer.flush()?;
    }

    if !args.skip_key_seed_errors && args.sketch == Sketch::Cms {
        let mut writer = KeySeedErrorCsvWriter::create(&args.output_key_seed_errors, false)?;
        if matches(impl_filter, config::cms_impl::DATASKETCHES) {
            cms::datasketches::write_key_seed_errors(
                &baseline,
                &mut writer,
                args.seed_filter,
                args.cols_filter,
            )?;
        }
        if matches(impl_filter, config::cms_impl::OXIDE) {
            cms::oxide::write_key_seed_errors(
                &baseline,
                &mut writer,
                args.seed_filter,
                args.cols_filter,
            )?;
        }
        if matches(impl_filter, config::cms_impl::SKETCHLIB) {
            cms::sketchlib::write_key_seed_errors(
                &baseline,
                &mut writer,
                args.seed_filter,
                args.cols_filter,
            )?;
        }
        writer.flush()?;
    }

    Ok(())
}

/// `true` if the `--impl` filter selects this implementation:
/// no filter → all impls run; explicit filter must match.
fn matches(filter: Option<&str>, candidate: &str) -> bool {
    match filter {
        None => true,
        Some(f) => f == candidate,
    }
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut sketch = Sketch::Cms;
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/frequency_accuracy_results.csv");
    let mut output_key_errors =
        PathBuf::from("../output/frequency_accuracy_key_median_errors.csv");
    let mut output_key_seed_errors =
        PathBuf::from("../output/frequency_accuracy_key_seed_errors.csv");
    let mut skip_summary = false;
    let mut skip_key_errors = false;
    let mut skip_key_seed_errors = false;
    let mut seed_filter = None;
    let mut cols_filter = None;
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
                    "Usage: frequency_accuracy --sketch cms|countsketch [--data PATH] \
                     [--output-summary PATH] [--output-key-errors PATH] \
                     [--output-key-seed-errors PATH] [--skip-summary] \
                     [--skip-key-errors] [--skip-key-seed-errors] \
                     [--seed N] [--cols N] [--impl NAME]"
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
