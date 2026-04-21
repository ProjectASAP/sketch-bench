use polars::prelude::*;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_DATA_PATH: &str = "../input/benchmark_data_10m_int64_zipf_s11_k500000.bin";
const QUERY_SNIPPET: &str =
    r#"df.lazy().select([col("v").n_unique().alias("exact_cardinality")])"#;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    repeats: usize,
    show_profile: bool,
    show_details: bool,
}

#[derive(Clone, Copy, Debug)]
struct Timings {
    load_dataset: u128,
    build_dataframe: u128,
    build_lazy_select: u128,
    result_total: u128,
    result_collect: u128,
    estimate_extract: u128,
    collect_schema: u128,
    explain_optimized: u128,
    to_alp_optimized_ir: u128,
    profile_execute: u128,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;

    println!("query.kind=polars_cardinality");
    println!("query.snippet={QUERY_SNIPPET}");
    println!("data={}", args.data.display());
    println!("repeats={}", args.repeats);

    for repeat in 1..=args.repeats {
        println!();
        println!("repeat={repeat}");
        run_probe(&args)?;
    }

    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from(DEFAULT_DATA_PATH);
    let mut repeats = 1usize;
    let mut show_profile = false;
    let mut show_details = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--repeats" => repeats = args.next().ok_or("--repeats requires a value")?.parse()?,
            "--profile" => show_profile = true,
            "--details" => show_details = true,
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release --bin polars_cardinality_probe -- \
[--data PATH] [--repeats N] [--profile] [--details]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        repeats,
        show_profile,
        show_details,
    })
}

/// Run one probe iteration and report the cost of:
/// 1. loading/building the input frame,
/// 2. building and executing the lazy `n_unique` query,
/// 3. extracting the scalar estimate from the one-row result,
/// 4. optional planning/profile inspection helpers.
fn run_probe(args: &Args) -> Result<(), Box<dyn Error>> {
    let load_started = Instant::now();
    let values = load_i64_dataset(&args.data)?;
    let load_elapsed = load_started.elapsed().as_nanos();

    let df_started = Instant::now();
    let df = build_i64_df(&values)?;
    let df_elapsed = df_started.elapsed().as_nanos();

    // This is the exact query path we want to understand:
    // build the lazy select, collect the one-row result, then read the scalar.
    let result_started = Instant::now();

    let lazy_started = Instant::now();
    let lazy = df
        .clone()
        .lazy()
        .select([col("v").n_unique().alias("exact_cardinality")]);
    let lazy_elapsed = lazy_started.elapsed().as_nanos();

    let collect_started = Instant::now();
    let result = lazy.clone().collect()?;
    let collect_elapsed = collect_started.elapsed().as_nanos();
    let result_elapsed = result_started.elapsed().as_nanos();

    let estimate_started = Instant::now();
    let estimate = extract_cardinality(&result)?;
    let estimate_elapsed = estimate_started.elapsed().as_nanos();
    let rows = values.len();

    // The remaining probes inspect planning / profiling overheads separately from
    // the main query path above.
    let mut schema_lazy = lazy.clone();
    let schema_started = Instant::now();
    let schema = schema_lazy.collect_schema()?;
    let schema_elapsed = schema_started.elapsed().as_nanos();

    let explain_started = Instant::now();
    let explain_optimized = lazy.clone().explain(true)?;
    let explain_elapsed = explain_started.elapsed().as_nanos();

    let alp_started = Instant::now();
    let alp = lazy.clone().to_alp_optimized()?;
    let alp_elapsed = alp_started.elapsed().as_nanos();
    std::hint::black_box(&alp);

    let profile_started = Instant::now();
    let (_profile_out, profile_df) = lazy.profile()?;
    let profile_elapsed = profile_started.elapsed().as_nanos();

    let timings = Timings {
        load_dataset: load_elapsed,
        build_dataframe: df_elapsed,
        build_lazy_select: lazy_elapsed,
        result_total: result_elapsed,
        result_collect: collect_elapsed,
        estimate_extract: estimate_elapsed,
        collect_schema: schema_elapsed,
        explain_optimized: explain_elapsed,
        to_alp_optimized_ir: alp_elapsed,
        profile_execute: profile_elapsed,
    };

    println!("rows={rows}");
    println!("result.exact_cardinality={estimate}");
    print_summary(timings);
    print_raw_timings(timings);

    if args.show_details {
        println!("[DETAIL] schema={schema:?}");
        println!("[DETAIL] result_shape={:?}", result.shape());
        println!("[DETAIL] optimized_plan:");
        println!("{explain_optimized}");
    }

    if args.show_profile {
        println!("[PROFILE] profile_df:");
        println!("{profile_df}");
    }

    Ok(())
}

fn print_summary(timings: Timings) {
    println!(
        "[SUMMARY] full_query_collect_ns={} (= load_dataset + build_dataframe + result_total = {} + {} + {})",
        timings.load_dataset
            + timings.build_dataframe
            + timings.result_total,
        timings.load_dataset,
        timings.build_dataframe,
        timings.result_total
    );
    println!(
        "[SUMMARY] query_breakdown_ns={} (= build_lazy_select + result_collect + estimate_extract = {} + {} + {})",
        timings.build_lazy_select + timings.result_collect + timings.estimate_extract,
        timings.build_lazy_select,
        timings.result_collect,
        timings.estimate_extract
    );
    println!(
        "[SUMMARY] planning_probe_ns={} (= build_lazy_select + collect_schema + explain_optimized + to_alp_optimized_ir = {} + {} + {} + {})",
        timings.build_lazy_select
            + timings.collect_schema
            + timings.explain_optimized
            + timings.to_alp_optimized_ir,
        timings.build_lazy_select,
        timings.collect_schema,
        timings.explain_optimized,
        timings.to_alp_optimized_ir
    );
    println!(
        "[SUMMARY] full_query_profile_ns={} (= load_dataset + build_dataframe + build_lazy_select + profile_execute = {} + {} + {} + {})",
        timings.load_dataset
            + timings.build_dataframe
            + timings.build_lazy_select
            + timings.profile_execute,
        timings.load_dataset,
        timings.build_dataframe,
        timings.build_lazy_select,
        timings.profile_execute
    );
}

fn print_raw_timings(timings: Timings) {
    println!("[TIMING] load_dataset_ns={}", timings.load_dataset);
    println!("[TIMING] build_dataframe_ns={}", timings.build_dataframe);
    println!("[TIMING] build_lazy_select_ns={}", timings.build_lazy_select);
    println!("[TIMING] result_total_ns={}", timings.result_total);
    println!("[TIMING] result_collect_ns={}", timings.result_collect);
    println!("[TIMING] estimate_extract_ns={}", timings.estimate_extract);
    println!("[TIMING] collect_schema_ns={}", timings.collect_schema);
    println!("[TIMING] explain_optimized_ns={}", timings.explain_optimized);
    println!(
        "[TIMING] to_alp_optimized_ir_ns={}",
        timings.to_alp_optimized_ir
    );
    println!("[TIMING] profile_execute_ns={}", timings.profile_execute);
}

/// Read the scalar result out of the one-row DataFrame using the same cast path
/// as the throughput query so the two probes stay directly comparable.
fn extract_cardinality(result: &DataFrame) -> Result<f64, Box<dyn Error>> {
    Ok(result
        .column("exact_cardinality")?
        .as_materialized_series()
        .cast(&DataType::Float64)?
        .f64()?
        .get(0)
        .unwrap_or(f64::NAN))
}

/// Load the binary benchmark input as little-endian i64 values.
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

/// Build the single-column DataFrame used by the cardinality query.
fn build_i64_df(values: &[i64]) -> Result<DataFrame, Box<dyn Error>> {
    Ok(DataFrame::new(vec![Column::new("v".into(), values)])?)
}
