use datasketches::hll::{HllSketch, HllType};
use sketch_oxide::cardinality::HyperLogLog as OxideHyperLogLog;
use asap_sketchlib::{ErtlMLE, HyperLogLogHIPP12, HyperLogLogP12, SketchInput};
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const LG_K: u8 = 12;
const REGISTERS: usize = 1 << (LG_K as usize);
const RUNS: usize = 10;
const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_hll";
const IMPLEMENTATION_RUST_SKETCHLIB_HIP: &str = "rust_sketchlib_hll_hip";
const IMPLEMENTATION_RUST_OXIDE: &str = "rust_oxide_hll";
const IMPLEMENTATION_RUST_DATASKETCHES: &str = "rust_datasketches_hll";
const CSV_HEADER: &str =
    "implementation,language,run,lg_k,registers,total_items,total_nanoseconds,throughput_items_per_sec";

#[derive(Clone, Debug)]
struct ThroughputRow {
    implementation: &'static str,
    language: &'static str,
    run: usize,
    lg_k: u8,
    registers: usize,
    total_items: usize,
    total_nanoseconds: u128,
    throughput_items_per_sec: f64,
}

impl ThroughputRow {
    fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.6}",
            self.implementation,
            self.language,
            self.run,
            self.lg_k,
            self.registers,
            self.total_items,
            self.total_nanoseconds,
            self.throughput_items_per_sec
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
            rows.extend(run_sketchlib_hip(&data));
            rows.extend(run_oxide(&data));
            rows.extend(run_datasketches(&data));
        }
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => rows.extend(run_sketchlib(&data)),
        Some(IMPLEMENTATION_RUST_SKETCHLIB_HIP) => rows.extend(run_sketchlib_hip(&data)),
        Some(IMPLEMENTATION_RUST_OXIDE) => rows.extend(run_oxide(&data)),
        Some(IMPLEMENTATION_RUST_DATASKETCHES) => rows.extend(run_datasketches(&data)),
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected \
                 {IMPLEMENTATION_RUST_SKETCHLIB}, {IMPLEMENTATION_RUST_SKETCHLIB_HIP}, \
                 {IMPLEMENTATION_RUST_OXIDE}, or {IMPLEMENTATION_RUST_DATASKETCHES}"
            )
            .into())
        }
    }

    write_rows(&args.output, &rows)?;
    Ok(())
}

fn run_sketchlib(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let mut sketch = HyperLogLogP12::<ErtlMLE>::default();
        let start = Instant::now();
        for &value in data {
            sketch.insert(&SketchInput::I64(value));
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_SKETCHLIB,
            language: "rust",
            run,
            lg_k: LG_K,
            registers: REGISTERS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn run_sketchlib_hip(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let mut sketch = HyperLogLogHIPP12::default();
        let start = Instant::now();
        for &value in data {
            sketch.insert(&SketchInput::I64(value));
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_SKETCHLIB_HIP,
            language: "rust",
            run,
            lg_k: LG_K,
            registers: REGISTERS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn run_oxide(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let mut sketch = OxideHyperLogLog::new(LG_K).expect("valid precision");
        let start = Instant::now();
        for &value in data {
            sketch.update(&value);
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_OXIDE,
            language: "rust",
            run,
            lg_k: LG_K,
            registers: REGISTERS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn run_datasketches(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let mut sketch = HllSketch::new(LG_K, HllType::Hll8);
        let start = Instant::now();
        for &value in data {
            sketch.update(value);
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_DATASKETCHES,
            language: "rust",
            run,
            lg_k: LG_K,
            registers: REGISTERS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/hll_throughput_results_rust.csv");
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--impl" => implementation_filter = Some(args.next().ok_or("--impl requires a value")?),
            "--help" | "-h" => {
                println!("Usage: cargo run --release -- [--data PATH] [--output PATH] [--impl NAME]");
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

fn write_rows(path: &Path, rows: &[ThroughputRow]) -> Result<(), Box<dyn Error>> {
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
