// Polars (exact reference) query throughput for CS-32K comparison.

use polars::prelude::*;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const ROWS: usize = 5;
const COLS: usize = 32768;
const SEEDS: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
const IMPLEMENTATION: &str = "polars_cs32k";
const CSV_HEADER: &str =
    "implementation,language,seed,rows,cols,total_items,total_queries,total_nanoseconds,throughput_queries_per_sec";

struct Dataset { values: Vec<i64>, keys: Vec<i64> }

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let Dataset { values, keys } = load_dataset(&args.data)?;
    let data_col = Column::new("v".into(), &values);
    let df = DataFrame::new(vec![data_col])?;
    let counts = df.lazy().group_by([col("v")]).agg([len().alias("count")]).collect()?;

    let mut csv_lines: Vec<String> = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let keys_col = Column::new("v".into(), &keys);
        let keys_df = DataFrame::new(vec![keys_col])?;
        let start = Instant::now();
        let result = keys_df.lazy().join(counts.clone().lazy(), [col("v")], [col("v")], JoinArgs::new(JoinType::Left)).collect()?;
        std::hint::black_box(&result);
        let elapsed = start.elapsed().as_nanos();
        let throughput = keys.len() as f64 * 1_000_000_000.0 / elapsed as f64;
        csv_lines.push(format!(
            "{},{},{},{},{},{},{},{},{:.6}",
            IMPLEMENTATION, "rust", seed, ROWS, COLS, values.len(), keys.len(), elapsed, throughput
        ));
    }
    write_csv(&args.output, CSV_HEADER, &csv_lines)?;
    Ok(())
}

#[derive(Debug)]
struct Args { data: PathBuf, output: PathBuf }

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/cs32k_throughput_query_results_polars.csv");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => { println!("Usage: cs32k_polars_throughput_query [--data PATH] [--output PATH]"); std::process::exit(0); }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    Ok(Args { data, output })
}

fn load_dataset(path: &Path) -> Result<Dataset, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size == 0 || file_size % 8 != 0 { return Err(format!("bad dataset size: {}", path.display()).into()); }
    let mut buffer = vec![0u8; file_size];
    file.read_exact(&mut buffer)?;
    let mut values = Vec::with_capacity(file_size / 8);
    let mut seen: HashSet<i64> = HashSet::with_capacity(file_size / 8);
    let mut keys: Vec<i64> = Vec::new();
    for chunk in buffer.chunks_exact(8) {
        let v = i64::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7]]);
        values.push(v);
        if seen.insert(v) { keys.push(v); }
    }
    Ok(Dataset { values, keys })
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
