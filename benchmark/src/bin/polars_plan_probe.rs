use polars::prelude::*;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_DATA_PATH: &str = "../input/benchmark_data_10m_int64_zipf_s11_k500000.bin";
const NUM_PERCENTILES: usize = 101;
type ProbeResult<T> = Result<T, Box<dyn Error>>;

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
    lazy_result_collect: u128,
    lazy_result_read: u128,
    eager_result_collect: u128,
    eager_result_read: u128,
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
        |df| {
            df.lazy()
                .select([col("v").n_unique().alias("exact_cardinality")])
        },
        exact_cardinality_eager,
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
        approx_cardinality_eager,
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
        quantile_eager,
        args,
    )?;

    run_int_probe(
        ProbeSpec {
            kind: "frequency",
            snippet: r#"df.lazy().group_by([col("v")]).agg([len().alias("count")])"#,
        },
        &ints,
        load_elapsed,
        |df| df.lazy().group_by([col("v")]).agg([len().alias("count")]),
        frequency_eager,
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
        cardinality_plus_quantile_eager,
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
            let grouped = df.lazy().group_by([col("v")]).agg([len().alias("count")]);
            global.cross_join(grouped, None)
        },
        cardinality_plus_frequency_eager,
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
            let grouped = df.lazy().group_by([col("v")]).agg([len().alias("count")]);
            global.cross_join(grouped, None)
        },
        quantile_plus_frequency_eager,
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
            let grouped = df.lazy().group_by([col("v")]).agg([len().alias("count")]);
            global.cross_join(grouped, None)
        },
        cardinality_plus_quantile_plus_frequency_eager,
        args,
    )?;

    Ok(())
}

fn run_int_probe<F, G>(
    spec: ProbeSpec,
    values: &[i64],
    load_elapsed: u128,
    build_lazy: F,
    run_eager: G,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
    G: Fn(DataFrame) -> ProbeResult<DataFrame>,
{
    let df_started = Instant::now();
    let df = build_i64_df(values)?;
    let df_elapsed = df_started.elapsed().as_nanos();

    run_probe(
        spec,
        df,
        load_elapsed,
        df_elapsed,
        build_lazy,
        run_eager,
        args,
    )
}

fn run_float_probe<F, G>(
    spec: ProbeSpec,
    values: &[f64],
    load_elapsed: u128,
    build_lazy: F,
    run_eager: G,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
    G: Fn(DataFrame) -> ProbeResult<DataFrame>,
{
    let df_started = Instant::now();
    let df = build_f64_df(values)?;
    let df_elapsed = df_started.elapsed().as_nanos();

    run_probe(
        spec,
        df,
        load_elapsed,
        df_elapsed,
        build_lazy,
        run_eager,
        args,
    )
}

fn run_probe<F, G>(
    spec: ProbeSpec,
    df: DataFrame,
    load_elapsed: u128,
    df_elapsed: u128,
    build_lazy: F,
    run_eager: G,
    args: &Args,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(DataFrame) -> LazyFrame,
    G: Fn(DataFrame) -> ProbeResult<DataFrame>,
{
    println!();
    println!("************************************************************");
    println!("query.kind={}", spec.kind);
    println!("query.snippet={}", spec.snippet);
    println!("************************************************************");
    println!();

    let lazy_started = Instant::now();
    let lazy = build_lazy(df.clone());
    let lazy_elapsed = lazy_started.elapsed().as_nanos();

    let collect_started = Instant::now();
    let lazy_result = lazy.clone().collect()?;
    let lazy_collect_elapsed = collect_started.elapsed().as_nanos();

    let lazy_read_started = Instant::now();
    read_probe_result(spec.kind, &lazy_result)?;
    let lazy_read_elapsed = lazy_read_started.elapsed().as_nanos();

    let eager_started = Instant::now();
    let eager_result = run_eager(df.clone())?;
    let eager_collect_elapsed = eager_started.elapsed().as_nanos();

    let eager_read_started = Instant::now();
    read_probe_result(spec.kind, &eager_result)?;
    let eager_read_elapsed = eager_read_started.elapsed().as_nanos();

    let lazy_result_schema = lazy_result.schema().clone();
    let eager_result_schema = eager_result.schema().clone();
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
        lazy_result_collect: lazy_collect_elapsed,
        lazy_result_read: lazy_read_elapsed,
        eager_result_collect: eager_collect_elapsed,
        eager_result_read: eager_read_elapsed,
    };

    print_summary(timings);
    print_raw_timings(timings);

    if args.show_details {
        println!("[DETAIL] input_df_shape={:?}", df.shape());
        println!("[DETAIL] input_df_schema={:?}", df.schema());
        println!("[DETAIL] lazy_result_shape={:?}", lazy_result.shape());
        println!("[DETAIL] lazy_result_schema={lazy_result_schema:?}");
        println!("[DETAIL] eager_result_shape={:?}", eager_result.shape());
        println!("[DETAIL] eager_result_schema={eager_result_schema:?}");
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
        "[SUMMARY] lazy_full_query_collect_ms={:.3} (= load_dataset + input_df_build + lazy_plan_build + lazy_result_collect = {:.3} + {:.3} + {:.3} + {:.3})",
        ns_to_ms(
            timings.load_dataset
                + timings.input_df_build
                + timings.lazy_plan_build
                + timings.lazy_result_collect,
        ),
        ns_to_ms(timings.load_dataset),
        ns_to_ms(timings.input_df_build),
        ns_to_ms(timings.lazy_plan_build),
        ns_to_ms(timings.lazy_result_collect),
    );
    println!(
        "[SUMMARY] eager_full_query_collect_ms={:.3} (= load_dataset + input_df_build + eager_result_collect = {:.3} + {:.3} + {:.3})",
        ns_to_ms(timings.load_dataset + timings.input_df_build + timings.eager_result_collect),
        ns_to_ms(timings.load_dataset),
        ns_to_ms(timings.input_df_build),
        ns_to_ms(timings.eager_result_collect),
    );
    println!(
        "[SUMMARY] lazy_full_query_plus_read_ms={:.3} (= load_dataset + input_df_build + lazy_plan_build + lazy_result_collect + lazy_result_read = {:.3} + {:.3} + {:.3} + {:.3} + {:.3})",
        ns_to_ms(
            timings.load_dataset
                + timings.input_df_build
                + timings.lazy_plan_build
                + timings.lazy_result_collect
                + timings.lazy_result_read,
        ),
        ns_to_ms(timings.load_dataset),
        ns_to_ms(timings.input_df_build),
        ns_to_ms(timings.lazy_plan_build),
        ns_to_ms(timings.lazy_result_collect),
        ns_to_ms(timings.lazy_result_read),
    );
    println!(
        "[SUMMARY] eager_full_query_plus_read_ms={:.3} (= load_dataset + input_df_build + eager_result_collect + eager_result_read = {:.3} + {:.3} + {:.3} + {:.3})",
        ns_to_ms(
            timings.load_dataset
                + timings.input_df_build
                + timings.eager_result_collect
                + timings.eager_result_read,
        ),
        ns_to_ms(timings.load_dataset),
        ns_to_ms(timings.input_df_build),
        ns_to_ms(timings.eager_result_collect),
        ns_to_ms(timings.eager_result_read),
    );
}

fn print_raw_timings(timings: Timings) {
    println!(
        "[TIMING] load_dataset_ms={:.3}",
        ns_to_ms(timings.load_dataset)
    );
    println!(
        "[TIMING] input_df_build_ms={:.3}",
        ns_to_ms(timings.input_df_build)
    );
    println!(
        "[TIMING] lazy_plan_build_ms={:.3}",
        ns_to_ms(timings.lazy_plan_build)
    );
    println!(
        "[TIMING] lazy_result_collect_ms={:.3}",
        ns_to_ms(timings.lazy_result_collect)
    );
    println!(
        "[TIMING] lazy_result_read_ms={:.3}",
        ns_to_ms(timings.lazy_result_read)
    );
    println!(
        "[TIMING] eager_result_collect_ms={:.3}",
        ns_to_ms(timings.eager_result_collect)
    );
    println!(
        "[TIMING] eager_result_read_ms={:.3}",
        ns_to_ms(timings.eager_result_read)
    );
}

fn ns_to_ms(ns: u128) -> f64 {
    ns as f64 / 1_000_000.0
}

fn read_probe_result(kind: &str, df: &DataFrame) -> ProbeResult<()> {
    match kind {
        "exact_cardinality" => read_numeric_agg_result(df, &["exact_cardinality"]),
        "approx_cardinality" => read_numeric_agg_result(df, &["approx_cardinality"]),
        "quantile" => read_numeric_agg_result(df, &["p_0", "p_100"]),
        "cardinality_plus_quantile" => read_numeric_agg_result(
            df,
            &["exact_cardinality", "approx_cardinality", "p_0", "p_100"],
        ),
        "frequency" => read_grouped_result(df, &["v", COUNT_COLUMN_PLACEHOLDER]),
        "cardinality_plus_frequency" => read_grouped_result(
            df,
            &[
                "exact_cardinality",
                "approx_cardinality",
                "v",
                COUNT_COLUMN_PLACEHOLDER,
            ],
        ),
        "quantile_plus_frequency" => {
            read_grouped_result(df, &["p_0", "p_100", "v", COUNT_COLUMN_PLACEHOLDER])
        }
        "cardinality_plus_quantile_plus_frequency" => read_grouped_result(
            df,
            &[
                "exact_cardinality",
                "approx_cardinality",
                "p_0",
                "p_100",
                "v",
                COUNT_COLUMN_PLACEHOLDER,
            ],
        ),
        other => Err(format!("unknown probe kind for result read: {other}").into()),
    }
}

const COUNT_COLUMN_PLACEHOLDER: &str = "__count__";

fn read_numeric_agg_result(df: &DataFrame, columns: &[&str]) -> ProbeResult<()> {
    let _ = df.shape();
    let _ = df.schema();
    for name in columns {
        read_cell(df, name, 0)?;
    }
    Ok(())
}

fn read_grouped_result(df: &DataFrame, columns: &[&str]) -> ProbeResult<()> {
    let _ = df.shape();
    let _ = df.schema();
    let height = df.height();
    if height == 0 {
        return Ok(());
    }

    for row in [0, height - 1] {
        for name in columns {
            read_cell(df, name, row)?;
        }
    }
    Ok(())
}

fn read_cell(df: &DataFrame, name: &str, row: usize) -> ProbeResult<()> {
    let actual = resolve_column_name(df, name)?;
    let _ = df.column(actual)?.as_materialized_series().get(row)?;
    Ok(())
}

fn resolve_column_name<'a>(df: &'a DataFrame, name: &'a str) -> ProbeResult<&'a str> {
    if name == COUNT_COLUMN_PLACEHOLDER {
        if df.column("count").is_ok() {
            return Ok("count");
        }
        if df.column("v_count").is_ok() {
            return Ok("v_count");
        }
        return Err("expected grouped result to contain either `count` or `v_count`".into());
    }

    Ok(name)
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

fn exact_cardinality_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let exact = df.column("v")?.as_materialized_series().n_unique()? as u32;
    build_single_row_df(vec![Column::new("exact_cardinality".into(), [exact])])
}

fn approx_cardinality_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let approx = df.column("v")?.approx_n_unique()? as u32;
    build_single_row_df(vec![Column::new("approx_cardinality".into(), [approx])])
}

fn quantile_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let series = df
        .column("v")?
        .as_materialized_series()
        .cast(&DataType::Float64)?;
    build_single_row_df(build_quantile_columns(&series)?)
}

fn frequency_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    Ok(df.group_by(["v"])?.select(["v"]).count()?)
}

fn cardinality_plus_quantile_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let series = df.column("v")?.as_materialized_series();
    let float_series = series.cast(&DataType::Float64)?;
    let mut cols = vec![
        Column::new("exact_cardinality".into(), [series.n_unique()? as u32]),
        Column::new(
            "approx_cardinality".into(),
            [df.column("v")?.approx_n_unique()? as u32],
        ),
    ];
    cols.extend(build_quantile_columns(&float_series)?);
    build_single_row_df(cols)
}

fn cardinality_plus_frequency_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let exact = df.column("v")?.as_materialized_series().n_unique()? as u32;
    let approx = df.column("v")?.approx_n_unique()? as u32;
    let counts = frequency_eager(df)?;
    let len = counts_height(&counts);
    with_repeated_prefix(
        counts,
        vec![
            repeated_u32_column("exact_cardinality", exact, len),
            repeated_u32_column("approx_cardinality", approx, len),
        ],
    )
}

fn quantile_plus_frequency_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let float_series = df
        .column("v")?
        .as_materialized_series()
        .cast(&DataType::Float64)?;
    let counts = frequency_eager(df)?;
    let prefix = build_quantile_repeated_columns(&float_series, counts_height(&counts))?;
    with_repeated_prefix(counts, prefix)
}

fn cardinality_plus_quantile_plus_frequency_eager(df: DataFrame) -> ProbeResult<DataFrame> {
    let exact = df.column("v")?.as_materialized_series().n_unique()? as u32;
    let approx = df.column("v")?.approx_n_unique()? as u32;
    let float_series = df
        .column("v")?
        .as_materialized_series()
        .cast(&DataType::Float64)?;
    let counts = frequency_eager(df)?;
    let mut prefix = vec![
        repeated_u32_column("exact_cardinality", exact, counts_height(&counts)),
        repeated_u32_column("approx_cardinality", approx, counts_height(&counts)),
    ];
    prefix.extend(build_quantile_repeated_columns(
        &float_series,
        counts_height(&counts),
    )?);
    with_repeated_prefix(counts, prefix)
}

fn build_quantile_columns(series: &Series) -> ProbeResult<Vec<Column>> {
    let quantiles = series.f64()?;
    let mut cols = Vec::with_capacity(NUM_PERCENTILES);
    for percentile in 0..NUM_PERCENTILES {
        let rank = percentile as f64 / 100.0;
        let value = quantiles
            .quantile(rank, QuantileMethod::Linear)?
            .unwrap_or(f64::NAN);
        cols.push(Column::new(format!("p_{percentile}").into(), [value]));
    }
    Ok(cols)
}

fn build_quantile_repeated_columns(series: &Series, len: usize) -> ProbeResult<Vec<Column>> {
    let quantiles = series.f64()?;
    let mut cols = Vec::with_capacity(NUM_PERCENTILES);
    for percentile in 0..NUM_PERCENTILES {
        let rank = percentile as f64 / 100.0;
        let value = quantiles
            .quantile(rank, QuantileMethod::Linear)?
            .unwrap_or(f64::NAN);
        cols.push(Column::new(
            format!("p_{percentile}").into(),
            vec![value; len],
        ));
    }
    Ok(cols)
}

fn build_single_row_df(columns: Vec<Column>) -> ProbeResult<DataFrame> {
    Ok(DataFrame::new(columns)?)
}

fn with_repeated_prefix(df: DataFrame, prefix: Vec<Column>) -> ProbeResult<DataFrame> {
    let mut columns = prefix;
    columns.extend(df.get_columns().iter().cloned());
    Ok(DataFrame::new(columns)?)
}

fn repeated_u32_column(name: &str, value: u32, len: usize) -> Column {
    Column::new(name.into(), vec![value; len])
}

fn counts_height(df: &DataFrame) -> usize {
    df.height()
}
