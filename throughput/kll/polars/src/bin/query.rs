// Polars (exact reference) quantile query throughput for KLL comparison.

use polars::prelude::*;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const K: i32 = 200;
const RUNS: usize = 10;
const REPEATS_PER_RUN: usize = 10;
const NUM_PERCENTILES: usize = 101;
const IMPLEMENTATION: &str = "polars_kll";
const CSV_HEADER: &str =
    "implementation,language,run,k,total_items,repeat,percentile,call_index,nanoseconds,estimate";

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let data = load_dataset(&args.data)?;

    let col = Column::new("v".into(), &data);
    let series = col.as_materialized_series().clone();

    let total = RUNS * REPEATS_PER_RUN * NUM_PERCENTILES;
    let mut csv_lines: Vec<String> = Vec::with_capacity(total);
    for run in 1..=RUNS {
        let mut call_index: usize = 0;
        for repeat in 1..=REPEATS_PER_RUN {
            for p in 0..NUM_PERCENTILES {
                call_index += 1;
                let q_rank = p as f64 / 100.0;
                let start = Instant::now();
                let q_scalar = series.quantile_reduce(q_rank, QuantileMethod::Linear)?;
                let q_value = q_scalar.value().try_extract::<f64>().unwrap_or(f64::NAN);
                std::hint::black_box(&q_value);
                let elapsed = start.elapsed().as_nanos();
                csv_lines.push(format!(
                    "{},{},{},{},{},{},{},{},{},{:.6}",
                    IMPLEMENTATION, "rust", run, K, data.len(), repeat, p, call_index, elapsed, q_value
                ));
            }
        }
    }
    write_csv(&args.output, CSV_HEADER, &csv_lines)?;
    Ok(())
}

#[derive(Debug)]
struct Args { data: PathBuf, output: PathBuf }

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/kll_throughput_query_results_polars.csv");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => { println!("Usage: kll_polars_throughput_query [--data PATH] [--output PATH]"); std::process::exit(0); }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    Ok(Args { data, output })
}

fn load_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size == 0 || file_size % 8 != 0 { return Err(format!("bad dataset size: {}", path.display()).into()); }
    let mut buffer = vec![0u8; file_size];
    file.read_exact(&mut buffer)?;
    let mut values = Vec::with_capacity(file_size / 8);
    for chunk in buffer.chunks_exact(8) {
        values.push(i64::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7]]));
    }
    Ok(values)
}

fn write_csv(path: &Path, header: &str, lines: &[String]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
    let mut w = BufWriter::new(file);
    writeln!(w, "{header}")?;
    for line in lines { writeln!(w, "{line}")?; }
    w.flush()?;
    Ok(())
}
