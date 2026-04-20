use polars::prelude::*;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_DATA_PATH: &str = "../input/benchmark_data_10m_int64_zipf_s11_k500000.bin";
const NUM_PERCENTILES: usize = 101;

#[derive(Clone, Copy, Debug)]
enum Workload {
    Cardinality,
    Frequency,
    Quantile,
}

impl Workload {
    fn parse(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "cardinality" => Ok(Self::Cardinality),
            "frequency" => Ok(Self::Frequency),
            "quantile" => Ok(Self::Quantile),
            _ => Err(format!("unsupported workload: {value}").into()),
        }
    }
}

#[derive(Debug)]
struct Args {
    data: PathBuf,
    workload: Workload,
    repeats: usize,
    show_profile: bool,
    show_details: bool,
}

#[derive(Clone, Copy, Debug)]
struct Timings {
    build_lazy_dsl: u128,
    collect_schema: u128,
    explain_unoptimized: u128,
    explain_optimized: u128,
    to_alp_optimized_ir: u128,
    collect_execute: u128,
    profile_execute: u128,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let ints = load_i64_dataset(&args.data)?;
    let floats = ints.iter().map(|&v| v as f64).collect::<Vec<_>>();

    println!("data={}", args.data.display());
    println!("rows={}", ints.len());
    println!("workload={:?}", args.workload);
    println!("repeats={}", args.repeats);

    for repeat in 1..=args.repeats {
        println!();
        println!("repeat={repeat}");
        match args.workload {
            Workload::Cardinality => {
                probe_cardinality(&ints, args.show_profile, args.show_details)?
            }
            Workload::Frequency => {
                probe_frequency(&ints, args.show_profile, args.show_details)?
            }
            Workload::Quantile => {
                probe_quantile(&floats, args.show_profile, args.show_details)?
            }
        }
    }

    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from(DEFAULT_DATA_PATH);
    let mut workload = Workload::Cardinality;
    let mut repeats = 1usize;
    let mut show_profile = false;
    let mut show_details = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--workload" => {
                workload = Workload::parse(&args.next().ok_or("--workload requires a value")?)?
            }
            "--repeats" => repeats = args.next().ok_or("--repeats requires a value")?.parse()?,
            "--profile" => show_profile = true,
            "--details" => show_details = true,
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release --bin polars_plan_probe -- \
[--data PATH] [--workload cardinality|frequency|quantile] [--repeats N] [--profile] [--details]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        workload,
        repeats,
        show_profile,
        show_details,
    })
}

fn probe_cardinality(
    values: &[i64],
    show_profile: bool,
    show_details: bool,
) -> Result<(), Box<dyn Error>> {
    let df = build_i64_df(values)?;

    measure_lazy_pipeline(
        "cardinality",
        || {
            df.clone()
                .lazy()
                .select([col("v").n_unique().alias("exact_cardinality")])
        },
        show_profile,
        show_details,
    )
}

fn probe_frequency(
    values: &[i64],
    show_profile: bool,
    show_details: bool,
) -> Result<(), Box<dyn Error>> {
    let df = build_i64_df(values)?;

    measure_lazy_pipeline(
        "frequency",
        || {
            df.clone()
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
        },
        show_profile,
        show_details,
    )
}

fn probe_quantile(
    values: &[f64],
    show_profile: bool,
    show_details: bool,
) -> Result<(), Box<dyn Error>> {
    let df = build_f64_df(values)?;
    let quantile_exprs = build_quantile_exprs();

    measure_lazy_pipeline(
        "quantile",
        || df.clone().lazy().select(quantile_exprs.clone()),
        show_profile,
        show_details,
    )
}

fn measure_lazy_pipeline<F>(
    label: &str,
    build_lazy: F,
    show_profile: bool,
    show_details: bool,
) -> Result<(), Box<dyn Error>>
where
    F: Fn() -> LazyFrame,
{
    let build_started = Instant::now();
    let lazy = build_lazy();
    let build_elapsed = build_started.elapsed().as_nanos();

    let mut schema_lazy = lazy.clone();
    let schema_started = Instant::now();
    let schema = schema_lazy.collect_schema()?;
    let schema_elapsed = schema_started.elapsed().as_nanos();

    let explain_unopt_started = Instant::now();
    let explain_unopt = lazy.clone().explain(false)?;
    let explain_unopt_elapsed = explain_unopt_started.elapsed().as_nanos();

    let explain_opt_started = Instant::now();
    let explain_opt = lazy.clone().explain(true)?;
    let explain_opt_elapsed = explain_opt_started.elapsed().as_nanos();

    let alp_started = Instant::now();
    let alp = lazy.clone().to_alp_optimized()?;
    let alp_elapsed = alp_started.elapsed().as_nanos();
    std::hint::black_box(&alp);

    let collect_started = Instant::now();
    let out = lazy.clone().collect()?;
    let collect_elapsed = collect_started.elapsed().as_nanos();
    std::hint::black_box(&out);

    // This measures execution again, but gives per-node timing from Polars itself.
    let profile_started = Instant::now();
    let (_profile_out, profile_df) = lazy.profile()?;
    let profile_elapsed = profile_started.elapsed().as_nanos();

    let timings = Timings {
        build_lazy_dsl: build_elapsed,
        collect_schema: schema_elapsed,
        explain_unoptimized: explain_unopt_elapsed,
        explain_optimized: explain_opt_elapsed,
        to_alp_optimized_ir: alp_elapsed,
        collect_execute: collect_elapsed,
        profile_execute: profile_elapsed,
    };

    print_summary(label, timings);
    print_raw_timings(label, timings);

    if show_details {
        println!("[DETAIL] {label}.schema={schema:?}");
        println!("[DETAIL] {label}.result_shape={:?}", out.shape());
    }

    if show_profile {
        println!("[PROFILE] {label}.profile_df:");
        println!("{profile_df}");
    }

    if show_details {
        println!("[DETAIL] {label}.explain_unoptimized:");
        println!("{explain_unopt}");
        println!("[DETAIL] {label}.explain_optimized:");
        println!("{explain_opt}");
    }

    Ok(())
}

fn print_summary(label: &str, timings: Timings) {
    println!(
        "[SUMMARY] {label}.full_query_collect_ns={} (= build_lazy_dsl + collect_execute = {} + {})",
        timings.build_lazy_dsl + timings.collect_execute,
        timings.build_lazy_dsl,
        timings.collect_execute
    );
    println!(
        "[SUMMARY] {label}.full_query_profile_ns={} (= build_lazy_dsl + profile_execute = {} + {})",
        timings.build_lazy_dsl + timings.profile_execute,
        timings.build_lazy_dsl,
        timings.profile_execute
    );
    println!(
        "[SUMMARY] {label}.planning_probe_min_ns={} (= build_lazy_dsl + to_alp_optimized_ir = {} + {})",
        timings.build_lazy_dsl + timings.to_alp_optimized_ir,
        timings.build_lazy_dsl,
        timings.to_alp_optimized_ir
    );
    println!(
        "[SUMMARY] {label}.planning_probe_full_ns={} (= build_lazy_dsl + collect_schema + explain_optimized + to_alp_optimized_ir = {} + {} + {} + {})",
        timings.build_lazy_dsl
            + timings.collect_schema
            + timings.explain_optimized
            + timings.to_alp_optimized_ir,
        timings.build_lazy_dsl,
        timings.collect_schema,
        timings.explain_optimized,
        timings.to_alp_optimized_ir
    );
}

fn print_raw_timings(label: &str, timings: Timings) {
    println!("[TIMING] {label}.build_lazy_dsl_ns={}", timings.build_lazy_dsl);
    println!("[TIMING] {label}.collect_schema_ns={}", timings.collect_schema);
    println!(
        "[TIMING] {label}.explain_unoptimized_ns={}",
        timings.explain_unoptimized
    );
    println!(
        "[TIMING] {label}.explain_optimized_ns={}",
        timings.explain_optimized
    );
    println!(
        "[TIMING] {label}.to_alp_optimized_ir_ns={}",
        timings.to_alp_optimized_ir
    );
    println!(
        "[TIMING] {label}.collect_execute_ns={}",
        timings.collect_execute
    );
    println!(
        "[TIMING] {label}.profile_execute_ns={}",
        timings.profile_execute
    );
}

fn load_i64_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() || bytes.len() % std::mem::size_of::<i64>() != 0 {
        return Err(format!("bad dataset size: {}", path.display()).into());
    }

    let mut values = Vec::with_capacity(bytes.len() / std::mem::size_of::<i64>());
    for chunk in bytes.chunks_exact(std::mem::size_of::<i64>()) {
        values.push(i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]));
    }
    Ok(values)
}

fn build_i64_df(values: &[i64]) -> Result<DataFrame, Box<dyn Error>> {
    Ok(DataFrame::new(vec![Column::new("v".into(), values)])?)
}

fn build_f64_df(values: &[f64]) -> Result<DataFrame, Box<dyn Error>> {
    Ok(DataFrame::new(vec![Column::new("v".into(), values)])?)
}

fn build_quantile_exprs() -> Vec<Expr> {
    (0..NUM_PERCENTILES)
        .map(|percentile| {
            let rank = percentile as f64 / 100.0;
            col("v")
                .quantile(lit(rank), QuantileMethod::Linear)
                .alias(format!("p_{percentile}"))
        })
        .collect()
}
