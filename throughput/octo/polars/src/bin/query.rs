// Polars (exact reference) top-K query throughput for Octo comparison.
// Idiomatic polars top-K: group_by + count + sort desc + head(k).

use polars::prelude::*;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RUNS: usize = 10;
const TOP_K: u32 = 100;
const CSV_HEADER: &str =
    "sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec";

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let data = load_dataset(&args.data)?;

    // Pre-build the DataFrame once (analogue of sketch already built).
    let col = Column::new("v".into(), &data);
    let df = DataFrame::new(vec![col])?;

    let mut csv_lines: Vec<String> = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let start = Instant::now();
        let topk = df.clone().lazy()
            .group_by([col("v")])
            .agg([len().alias("count")])
            .sort(["count"], SortMultipleOptions::new().with_order_descending(true))
            .limit(TOP_K)
            .collect()?;
        std::hint::black_box(&topk);
        let elapsed = start.elapsed().as_nanos();
        let throughput = data.len() as f64 * 1e9 / elapsed as f64;
        csv_lines.push(format!(
            "{},{},{},{},{},{},{:.6}",
            "octo", "polars", 1, run, data.len(), elapsed, throughput
        ));
    }
    write_csv(&args.output, CSV_HEADER, &csv_lines)?;
    Ok(())
}

#[derive(Debug)]
struct Args { data: PathBuf, output: PathBuf }

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/octo_throughput_query_results_polars.csv");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => { println!("Usage: octo_polars_throughput_query [--data PATH] [--output PATH]"); std::process::exit(0); }
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
