use polars::prelude::*;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RUNS: usize = 10;
const NUM_PERCENTILES: usize = 101;
const IMPLEMENTATION: &str = "polars_quantile";
const K: i32 = 200;
const ALPHA: f64 = 0.01;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output: PathBuf,
    variant: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let config = variant_config(&args.variant)?;
    let data = load_dataset(&args.data)?;
    let quantile_exprs = build_quantile_exprs();
    let mut csv_lines: Vec<String> = Vec::with_capacity(RUNS);

    for run in 1..=RUNS {
        let start = Instant::now();
        let value_col = Column::new("v".into(), &data);
        let df = DataFrame::new(vec![value_col])?;
        let result = df.lazy().select(quantile_exprs.clone()).collect()?;
        std::hint::black_box(&result);
        let elapsed = start.elapsed().as_nanos();
        let throughput = data.len() as f64 * 1_000_000_000.0 / elapsed as f64;
        csv_lines.push(match config {
            VariantConfig::Kll => format!(
                "{},{},{},{},{},{},{:.6}",
                IMPLEMENTATION, "rust", run, K, data.len(), elapsed, throughput
            ),
            VariantConfig::Dd => format!(
                "{},{},{},{:.6},{},{},{:.6}",
                IMPLEMENTATION, "rust", run, ALPHA, data.len(), elapsed, throughput
            ),
        });
    }

    write_csv(&args.output, config.insert_header(), &csv_lines)?;
    Ok(())
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

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../kll/output/kll_throughput_results_polars.csv");
    let mut variant = String::from("kll");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--variant" => variant = args.next().ok_or("--variant requires a value")?,
            "--help" | "-h" => {
                println!(
                    "Usage: polars_quantile_throughput [--data PATH] [--output PATH] [--variant kll|dd]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    Ok(Args {
        data,
        output,
        variant,
    })
}

enum VariantConfig {
    Kll,
    Dd,
}

impl VariantConfig {
    fn insert_header(&self) -> &'static str {
        match self {
            Self::Kll => {
                "implementation,language,run,k,total_items,total_nanoseconds,throughput_items_per_sec"
            }
            Self::Dd => {
                "implementation,language,run,alpha,total_items,total_nanoseconds,throughput_items_per_sec"
            }
        }
    }
}

fn variant_config(variant: &str) -> Result<VariantConfig, Box<dyn Error>> {
    match variant {
        "kll" => Ok(VariantConfig::Kll),
        "dd" => Ok(VariantConfig::Dd),
        _ => Err(format!("unsupported polars_quantile variant: {variant}").into()),
    }
}

fn load_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size == 0 || file_size % 8 != 0 {
        return Err(format!("bad dataset size: {}", path.display()).into());
    }
    let mut buffer = vec![0u8; file_size];
    file.read_exact(&mut buffer)?;
    let mut values = Vec::with_capacity(file_size / 8);
    for chunk in buffer.chunks_exact(8) {
        values.push(i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]));
    }
    Ok(values)
}

fn write_csv(path: &Path, header: &str, lines: &[String]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let mut w = BufWriter::new(file);
    writeln!(w, "{header}")?;
    for line in lines {
        writeln!(w, "{line}")?;
    }
    w.flush()?;
    Ok(())
}
