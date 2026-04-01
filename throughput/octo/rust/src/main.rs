use sketchlib_rust::{Count, CountMin, FastPath, SketchInput, impl_fixed_matrix};

impl_fixed_matrix!(M5x32K, i32, 5, 32768);
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::time::Instant;

const RUNS: usize = 10;
const THREAD_COUNTS: [usize; 4] = [1, 2, 4, 8];

const CSV_HEADER: &str =
    "sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec";

#[derive(Clone, Debug)]
struct ThroughputRow {
    sketch_type: &'static str,
    implementation: &'static str,
    num_workers: usize,
    run: usize,
    total_items: usize,
    total_nanoseconds: u128,
    throughput_items_per_sec: f64,
}

impl ThroughputRow {
    fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{:.6}",
            self.sketch_type,
            self.implementation,
            self.num_workers,
            self.run,
            self.total_items,
            self.total_nanoseconds,
            self.throughput_items_per_sec,
        )
    }
}

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let data = load_dataset(&args.data)?;

    let inputs: Vec<SketchInput<'static>> =
        data.iter().map(|&v| SketchInput::I64(v)).collect();

    let mut rows = Vec::new();

    rows.extend(run_regular_cms(&inputs));
    rows.extend(run_octo_cms(&inputs));

    rows.extend(run_regular_cs(&inputs));
    rows.extend(run_octo_cs(&inputs));

    rows.extend(run_regular_hll(&inputs));
    rows.extend(run_octo_hll(&inputs));

    write_rows(&args.output, &rows)?;
    eprintln!("Wrote {}", args.output.display());
    Ok(())
}

fn partition<'a>(inputs: &'a [SketchInput<'static>], n: usize) -> Vec<&'a [SketchInput<'static>]> {
    let chunk_size = (inputs.len() + n - 1) / n;
    inputs.chunks(chunk_size).collect()
}

fn run_regular_cms(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let barrier = Barrier::new(1);
        let nanos = std::thread::scope(|s| {
            let h = s.spawn(|| {
                let mut sketch = CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
                barrier.wait();
                let start = Instant::now();
                for input in inputs {
                    sketch.insert_emit_delta(input, &mut |d| {
                        std::hint::black_box(&d);
                    });
                }
                std::hint::black_box(&sketch);
                start.elapsed().as_nanos()
            });
            h.join().unwrap()
        });
        rows.push(ThroughputRow {
            sketch_type: "cms",
            implementation: "regular",
            num_workers: 1,
            run,
            total_items: inputs.len(),
            total_nanoseconds: nanos,
            throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
        });
    }
    rows
}

fn run_octo_cms(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    let mut rows = Vec::new();
    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let barrier = Barrier::new(num_workers);
            let nanos = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        let barrier = &barrier;
                        s.spawn(move || {
                            let mut sketch = CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
                            barrier.wait();
                            let start = Instant::now();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| {
                                    std::hint::black_box(&d);
                                });
                            }
                            start.elapsed().as_nanos()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h: std::thread::ScopedJoinHandle<'_, u128>| h.join().unwrap())
                    .max()
                    .unwrap()
            });
            rows.push(ThroughputRow {
                sketch_type: "cms",
                implementation: "octo",
                num_workers,
                run,
                total_items: inputs.len(),
                total_nanoseconds: nanos,
                throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
            });
        }
    }
    rows
}

fn run_regular_cs(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let barrier = Barrier::new(1);
        let nanos = std::thread::scope(|s| {
            let h = s.spawn(|| {
                let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                barrier.wait();
                let start = Instant::now();
                for input in inputs {
                    sketch.insert_emit_delta(input, &mut |d| {
                        std::hint::black_box(&d);
                    });
                }
                std::hint::black_box(&sketch);
                start.elapsed().as_nanos()
            });
            h.join().unwrap()
        });
        rows.push(ThroughputRow {
            sketch_type: "cs",
            implementation: "regular",
            num_workers: 1,
            run,
            total_items: inputs.len(),
            total_nanoseconds: nanos,
            throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
        });
    }
    rows
}

fn run_octo_cs(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    let mut rows = Vec::new();
    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let barrier = Barrier::new(num_workers);
            let nanos = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        let barrier = &barrier;
                        s.spawn(move || {
                            let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                            barrier.wait();
                            let start = Instant::now();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| {
                                    std::hint::black_box(&d);
                                });
                            }
                            start.elapsed().as_nanos()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h: std::thread::ScopedJoinHandle<'_, u128>| h.join().unwrap())
                    .max()
                    .unwrap()
            });
            rows.push(ThroughputRow {
                sketch_type: "cs",
                implementation: "octo",
                num_workers,
                run,
                total_items: inputs.len(),
                total_nanoseconds: nanos,
                throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
            });
        }
    }
    rows
}

fn run_regular_hll(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    use sketchlib_rust::{HyperLogLog, Regular};
    let mut rows = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let barrier = Barrier::new(1);
        let nanos = std::thread::scope(|s| {
            let h = s.spawn(|| {
                let mut sketch = HyperLogLog::<Regular>::default();
                barrier.wait();
                let start = Instant::now();
                for input in inputs {
                    sketch.insert_emit_delta(input, &mut |d| {
                        std::hint::black_box(&d);
                    });
                }
                std::hint::black_box(&sketch);
                start.elapsed().as_nanos()
            });
            h.join().unwrap()
        });
        rows.push(ThroughputRow {
            sketch_type: "hll",
            implementation: "regular",
            num_workers: 1,
            run,
            total_items: inputs.len(),
            total_nanoseconds: nanos,
            throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
        });
    }
    rows
}

fn run_octo_hll(inputs: &[SketchInput<'static>]) -> Vec<ThroughputRow> {
    let mut rows = Vec::new();
    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let barrier = Barrier::new(num_workers);
            let nanos = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        let barrier = &barrier;
                        s.spawn(move || {
                            let mut sketch = sketchlib_rust::HyperLogLog::<sketchlib_rust::Regular>::default();
                            barrier.wait();
                            let start = Instant::now();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| {
                                    std::hint::black_box(&d);
                                });
                            }
                            start.elapsed().as_nanos()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h: std::thread::ScopedJoinHandle<'_, u128>| h.join().unwrap())
                    .max()
                    .unwrap()
            });
            rows.push(ThroughputRow {
                sketch_type: "hll",
                implementation: "octo",
                num_workers,
                run,
                total_items: inputs.len(),
                total_nanoseconds: nanos,
                throughput_items_per_sec: inputs.len() as f64 * 1e9 / nanos as f64,
            });
        }
    }
    rows
}

fn load_dataset(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size == 0 {
        return Err(format!("dataset is empty: {}", path.display()).into());
    }
    if file_size % 8 != 0 {
        return Err(format!("dataset size not divisible by 8: {}", path.display()).into());
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
    let mut w = BufWriter::new(file);
    writeln!(w, "{CSV_HEADER}")?;
    for row in rows {
        writeln!(w, "{}", row.to_csv_line())?;
    }
    w.flush()?;
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/octo_throughput_results.csv");

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output" => output = PathBuf::from(args.next().ok_or("--output requires a path")?),
            "--help" | "-h" => {
                println!("Usage: octo_throughput [--data PATH] [--output PATH]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args { data, output })
}
