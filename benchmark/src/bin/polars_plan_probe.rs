use polars::prelude::*;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_DATA_PATH: &str = "../input/benchmark_data_10m_int64_zipf_s11_k500000.bin";
const NUM_PERCENTILES: usize = 101;

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
    input_df_build: u128,
    lazy_plan_build: u128,
    result_collect: u128,
}

#[derive(Clone, Copy, Debug)]
struct ProbeSpec {
    kind: &'static str,
    snippet: &'static str,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;

    println!("probe.kind=polars_plan_probe");
    println!("data={}", args.data.display());
    println!("repeats={}", args.repeats);

    for repeat in 1..=args.repeats {
        println!();
        println!("repeat={repeat}");
        run_all_probes(&args)?;
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
                    "Usage: cargo run --release --bin polars_plan_probe -- \
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

fn run_all_probes(args: &Args) -> Result<(), Box<dyn Error>> {
    let load_started = Instant::now();
    let ints = load_i64_dataset(&args.data)?;
    let load_elapsed = load_started.elapsed().as_nanos();
    let floats = ints.iter().map(|&value| value as f64).collect::<Vec<_>>();

    println!("rows={}", ints.len());

    run_int_probe(
        ProbeSpec {
            kind: "exact_cardinality",
            snippet: r#"df.lazy().select([col("v").n_unique().alias("exact_cardinality")])"#,
        },
        &ints,
        load_elapsed,
        |df| df.lazy().select([col("v").n_unique().alias("exact_cardinality")]),
        args,
    )?;

    run_int_probe(
        ProbeSpec {
            kind: "approx_cardinality",
            snippet: r#"df.lazy().select([col("v").approx_n_unique().alias("approx_cardinality")])"#,
        },
        &ints,
        load_elapsed,
        |df| {
            df.lazy()
                .select([col("v").approx_n_unique().alias("approx_cardinality")])
        },
        args,
    )?;

    run_float_probe(
        ProbeSpec {
            kind: "quantile",
            snippet: r#"df.lazy().select(build_quantile_exprs())"#,
        },
        &floats,
        load_elapsed,
        |df| df.lazy().select(build_quantile_exprs()),
        args,
    )?;

    run_int_probe(
        ProbeSpec {
            kind: "frequency",
            snippet: r#"df.lazy().group_by([col("v")]).agg([len().alias("count")])"#,
        },
        &ints,
        load_elapsed,
        |df| {
            df.lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
                .sort(
                    ["count", "v"],
                    SortMultipleOptions::default().with_order_descending_multi([true, false]),
                )
        },
        args,
    )?;

    run_float_probe(
        ProbeSpec {
            kind: "cardinality_plus_quantile",
            snippet: r#"df.lazy().select([col("v").n_unique().alias("exact_cardinality"), col("v").approx_n_unique().alias("approx_cardinality"), build_quantile_exprs()...])"#,
        },
        &floats,
        load_elapsed,
        |df| {
            let mut exprs = vec![
                col("v").n_unique().alias("exact_cardinality"),
                col("v").approx_n_unique().alias("approx_cardinality"),
            ];
            exprs.extend(build_quantile_exprs());
            df.lazy().select(exprs)
        },
        args,
    )?;

    run_int_probe(
        ProbeSpec {
            kind: "cardinality_plus_frequency",
            snippet: r#"global_cardinality.cross_join(grouped_frequency)"#,
        },
        &ints,
        load_elapsed,
        |df| {
            let global = df.clone().lazy().select([
                col("v").n_unique().alias("exact_cardinality"),
                col("v").approx_n_unique().alias("approx_cardinality"),
            ]);
            let grouped = df
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
                .sort(
                    ["count", "v"],
                    SortMultipleOptions::default().with_order_descending_multi([true, false]),
                );
            global.cross_join(grouped, None)
        },
        args,
    )?;

    run_float_probe(
        ProbeSpec {
            kind: "quantile_plus_frequency",
            snippet: r#"global_quantiles.cross_join(grouped_frequency)"#,
        },
        &floats,
        load_elapsed,
        |df| {
            let global = df.clone().lazy().select(build_quantile_exprs());
            let grouped = df
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
                .sort(
                    ["count", "v"],
                    SortMultipleOptions::default().with_order_descending_multi([true, false]),
                );
            global.cross_join(grouped, None)
        },
        args,
    )?;

    run_float_probe(
        ProbeSpec {
            kind: "cardinality_plus_quantile_plus_frequency",
            snippet: r#"global_cardinality_and_quantiles.cross_join(grouped_frequency)"#,
        },
        &floats,
        load_elapsed,
        |df| {
            let mut exprs = vec![
                col("v").n_unique().alias("exact_cardinality"),
                col("v").approx_n_unique().alias("approx_cardinality"),
            ];
            exprs.extend(build_quantile_exprs());
            let global = df.clone().lazy().select(exprs);
            let grouped = df
                .lazy()
                .group_by([col("v")])
                .agg([len().alias("count")])
                .sort(
                    ["count", "v"],
                    SortMultipleOptions::default().with_order_descending_multi([true, false]),
                );
            global.cross_join(grouped, None)
        },
        args,
    )?;

    Ok(())
}

fn run_int_probe<F>(
    spec: ProbeSpec,
    values: &[i64],
    load_elapsed: u128,
    build_lazy: F,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
{
    let df_started = Instant::now();
    let df = build_i64_df(values)?;
    let df_elapsed = df_started.elapsed().as_nanos();

    run_probe(spec, df, load_elapsed, df_elapsed, build_lazy, args)
}

fn run_float_probe<F>(
    spec: ProbeSpec,
    values: &[f64],
    load_elapsed: u128,
    build_lazy: F,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
{
    let df_started = Instant::now();
    let df = build_f64_df(values)?;
    let df_elapsed = df_started.elapsed().as_nanos();

    run_probe(spec, df, load_elapsed, df_elapsed, build_lazy, args)
}

fn run_probe<F>(
    spec: ProbeSpec,
    df: DataFrame,
    load_elapsed: u128,
    df_elapsed: u128,
    build_lazy: F,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
{
    println!();
    println!("query.kind={}", spec.kind);
    println!("query.snippet={}", spec.snippet);

    let lazy_started = Instant::now();
    let lazy = build_lazy(df.clone());
    let lazy_elapsed = lazy_started.elapsed().as_nanos();

    let collect_started = Instant::now();
    let result = lazy.clone().collect()?;
    let collect_elapsed = collect_started.elapsed().as_nanos();

    let result_schema = result.schema().clone();
    let lazy_plan_unoptimized = if args.show_details {
        Some(lazy.clone().explain(false)?)
    } else {
        None
    };
    let lazy_plan_optimized = if args.show_details {
        Some(lazy.clone().explain(true)?)
    } else {
        None
    };
    let profile_df = if args.show_profile {
        Some(lazy.profile()?.1)
    } else {
        None
    };

    let timings = Timings {
        load_dataset: load_elapsed,
        input_df_build: df_elapsed,
        lazy_plan_build: lazy_elapsed,
        result_collect: collect_elapsed,
    };

    print_summary(timings);
    print_raw_timings(timings);

    if args.show_details {
        println!("[DETAIL] input_df_shape={:?}", df.shape());
        println!("[DETAIL] input_df_schema={:?}", df.schema());
        println!("[DETAIL] input_df_head:");
        println!("{:?}", df.head(Some(5)));
        println!("[DETAIL] result_shape={:?}", result.shape());
        println!("[DETAIL] result_schema={result_schema:?}");
        println!("[DETAIL] result_head:");
        println!("{:?}", result.head(Some(5)));
        println!("[DETAIL] lazy_plan_unoptimized:");
        println!("{}", lazy_plan_unoptimized.as_deref().unwrap_or(""));
        println!("[DETAIL] lazy_plan_optimized:");
        println!("{}", lazy_plan_optimized.as_deref().unwrap_or(""));
    }

    if args.show_profile {
        println!("[PROFILE] profile_df:");
        println!("{}", profile_df.unwrap());
    }

    Ok(())
}

fn print_summary(timings: Timings) {
    println!(
        "[SUMMARY] full_query_collect_ns={} (= load_dataset + input_df_build + lazy_plan_build + result_collect = {} + {} + {} + {})",
        timings.load_dataset
            + timings.input_df_build
            + timings.lazy_plan_build
            + timings.result_collect,
        timings.load_dataset,
        timings.input_df_build,
        timings.lazy_plan_build,
        timings.result_collect,
    );
}

fn print_raw_timings(timings: Timings) {
    println!("[TIMING] load_dataset_ns={}", timings.load_dataset);
    println!("[TIMING] input_df_build_ns={}", timings.input_df_build);
    println!("[TIMING] lazy_plan_build_ns={}", timings.lazy_plan_build);
    println!("[TIMING] result_collect_ns={}", timings.result_collect);
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
