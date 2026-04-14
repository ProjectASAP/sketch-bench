use sketch_oxide::quantiles::KllSketch as OxideKll;
use asap_sketchlib::KLL;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const K: i32 = 200;
const RUNS: usize = 10;
const REPEATS_PER_RUN: usize = 10;
const NUM_PERCENTILES: usize = 101; // p0..p100 inclusive
const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_kll";
const IMPLEMENTATION_RUST_OXIDE: &str = "rust_oxide_kll";
const CSV_HEADER: &str =
    "implementation,language,run,k,total_items,repeat,percentile,call_index,nanoseconds,estimate";

#[derive(Clone, Debug)]
struct QueryRow {
    implementation: &'static str,
    language: &'static str,
    run: usize,
    k: i32,
    total_items: usize,
    repeat: usize,
    percentile: usize,
    call_index: usize,
    nanoseconds: u128,
    estimate: f64,
}

impl QueryRow {
    fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{:.6}",
            self.implementation,
            self.language,
            self.run,
            self.k,
            self.total_items,
            self.repeat,
            self.percentile,
            self.call_index,
            self.nanoseconds,
            self.estimate
        )
    }
}

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output: PathBuf,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let data = load_dataset(&args.data)?;

    let mut rows = Vec::new();
    match args.implementation_filter.as_deref() {
        None => {
            rows.extend(run_sketchlib(&data));
            rows.extend(run_oxide(&data));
        }
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => rows.extend(run_sketchlib(&data)),
        Some(IMPLEMENTATION_RUST_OXIDE) => rows.extend(run_oxide(&data)),
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected \
                 {IMPLEMENTATION_RUST_SKETCHLIB} or {IMPLEMENTATION_RUST_OXIDE}"
            )
            .into())
        }
    }

    write_rows(&args.output, &rows)?;
    Ok(())
}

fn run_sketchlib(data: &[i64]) -> Vec<QueryRow> {
    let total = RUNS * REPEATS_PER_RUN * NUM_PERCENTILES;
    let mut rows = Vec::with_capacity(total);
    for run in 1..=RUNS {
        let mut sketch: KLL<i64> = KLL::init_kll(K);
        for &value in data {
            sketch.update(&value).expect("KLL insert succeeds");
        }
        std::hint::black_box(&sketch);
        let mut call_index: usize = 0;
        for repeat in 1..=REPEATS_PER_RUN {
            for p in 0..NUM_PERCENTILES {
                call_index += 1;
                let rank = p as f64 / 100.0;
                let start = Instant::now();
                let q = sketch.quantile(rank);
                std::hint::black_box(&q);
                let elapsed = start.elapsed().as_nanos();
                rows.push(QueryRow {
                    implementation: IMPLEMENTATION_RUST_SKETCHLIB,
                    language: "rust",
                    run,
                    k: K,
                    total_items: data.len(),
                    repeat,
                    percentile: p,
                    call_index,
                    nanoseconds: elapsed,
                    estimate: q as f64,
                });
            }
        }
    }
    rows
}

fn run_oxide(data: &[i64]) -> Vec<QueryRow> {
    let total = RUNS * REPEATS_PER_RUN * NUM_PERCENTILES;
    let mut rows = Vec::with_capacity(total);
    for run in 1..=RUNS {
        let mut sketch = OxideKll::new(K as u16).expect("valid KLL k");
        for &value in data {
            sketch.update(value as f64);
        }
        std::hint::black_box(&sketch);
        let mut call_index: usize = 0;
        for repeat in 1..=REPEATS_PER_RUN {
            for p in 0..NUM_PERCENTILES {
                call_index += 1;
                let rank = p as f64 / 100.0;
                let start = Instant::now();
                let q = sketch.quantile(rank).expect("sketch is non-empty");
                std::hint::black_box(&q);
                let elapsed = start.elapsed().as_nanos();
                rows.push(QueryRow {
                    implementation: IMPLEMENTATION_RUST_OXIDE,
                    language: "rust",
                    run,
                    k: K,
                    total_items: data.len(),
                    repeat,
                    percentile: p,
                    call_index,
                    nanoseconds: elapsed,
                    estimate: q,
                });
            }
        }
    }
    rows
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/kll_throughput_query_results_rust.csv");
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--impl" => implementation_filter = Some(args.next().ok_or("--impl requires a value")?),
            "--help" | "-h" => {
                println!("Usage: kll_throughput_query [--data PATH] [--output PATH] [--impl NAME]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output,
        implementation_filter,
    })
}

fn load_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len() as usize;
    if file_size == 0 {
        return Err(format!("dataset is empty: {}", path.display()).into());
    }
    if file_size % std::mem::size_of::<i64>() != 0 {
        return Err(format!("dataset size is not divisible by 8 bytes: {}", path.display()).into());
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

fn write_rows(path: &Path, rows: &[QueryRow]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "{CSV_HEADER}")?;
    for row in rows {
        writeln!(writer, "{}", row.to_csv_line())?;
    }
    writer.flush()?;
    Ok(())
}
