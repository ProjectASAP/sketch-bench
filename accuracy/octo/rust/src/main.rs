use sketchlib_rust::{
    CmDelta, Count, CountDelta, CountMin, FastPath, HllDelta, HyperLogLog, Regular, SketchInput,
    impl_fixed_matrix,
};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

const ROWS: usize = 5;
const COLS: usize = 32768;
const HEAVY_HITTER_MIN_TRUE_COUNT: u64 = 100;
const THREAD_COUNTS: [usize; 4] = [1, 2, 4, 8];
const RUNS: usize = 10;

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_cms: PathBuf,
    output_cs: PathBuf,
    output_hll: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let (values, frequencies) = load_baseline(&args.data)?;
    let distinct = frequencies.len();

    let heavy_hitters: Vec<(i64, u64)> = frequencies
        .iter()
        .filter_map(|(&k, &v)| (v >= HEAVY_HITTER_MIN_TRUE_COUNT).then_some((k, v)))
        .collect();

    let inputs: Vec<SketchInput<'static>> =
        values.iter().map(|&v| SketchInput::I64(v)).collect();

    run_cms_accuracy(&args.output_cms, &inputs, &heavy_hitters, values.len())?;
    run_cs_accuracy(&args.output_cs, &inputs, &heavy_hitters, values.len())?;
    run_hll_accuracy(&args.output_hll, &inputs, distinct, values.len())?;
    Ok(())
}

fn partition<'a>(inputs: &'a [SketchInput<'static>], n: usize) -> Vec<&'a [SketchInput<'static>]> {
    let chunk_size = (inputs.len() + n - 1) / n;
    inputs.chunks(chunk_size).collect()
}

fn run_cms_accuracy(
    output: &Path,
    inputs: &[SketchInput<'static>],
    heavy_hitters: &[(i64, u64)],
    total_items: usize,
) -> Result<(), Box<dyn Error>> {
    let header = "implementation,num_workers,run,rows,cols,total_items,distinct_hh,avg_relative_error,max_relative_error,mean_absolute_error";
    let file = create_csv(output, header)?;
    let mut w = BufWriter::new(file);

    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let all_deltas: Vec<Vec<CmDelta>> = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        s.spawn(move || {
                            let mut sketch =
                                CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
                            let mut deltas = Vec::new();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| deltas.push(d));
                            }
                            deltas
                        })
                    })
                    .collect();
                handles.into_iter().map(|h: std::thread::ScopedJoinHandle<'_, Vec<CmDelta>>| h.join().unwrap()).collect()
            });

            let mut agg =
                CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
            for batch in &all_deltas {
                for &delta in batch {
                    agg.apply_delta(delta);
                }
            }

            let (avg_re, max_re, mae) = cms_errors(&agg, heavy_hitters);
            writeln!(
                w,
                "octo_cms,{num_workers},{run},{ROWS},{COLS},{total_items},{},{avg_re:.12},{max_re:.12},{mae:.12}",
                heavy_hitters.len(),
            )?;
        }
    }
    w.flush()?;
    eprintln!("Wrote {}", output.display());
    Ok(())
}

fn run_cs_accuracy(
    output: &Path,
    inputs: &[SketchInput<'static>],
    heavy_hitters: &[(i64, u64)],
    total_items: usize,
) -> Result<(), Box<dyn Error>> {
    let header = "implementation,num_workers,run,rows,cols,total_items,distinct_hh,avg_relative_error,max_relative_error,mean_absolute_error";
    let file = create_csv(output, header)?;
    let mut w = BufWriter::new(file);

    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let all_deltas: Vec<Vec<CountDelta>> = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        s.spawn(move || {
                            let mut sketch =
                                Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                            let mut deltas = Vec::new();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| deltas.push(d));
                            }
                            deltas
                        })
                    })
                    .collect();
                handles.into_iter().map(|h: std::thread::ScopedJoinHandle<'_, Vec<CountDelta>>| h.join().unwrap()).collect()
            });

            let mut agg =
                Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
            for batch in &all_deltas {
                for &delta in batch {
                    agg.apply_delta(delta);
                }
            }

            let (avg_re, max_re, mae) = cs_errors(&agg, heavy_hitters);
            writeln!(
                w,
                "octo_cs,{num_workers},{run},{ROWS},{COLS},{total_items},{},{avg_re:.12},{max_re:.12},{mae:.12}",
                heavy_hitters.len(),
            )?;
        }
    }
    w.flush()?;
    eprintln!("Wrote {}", output.display());
    Ok(())
}

fn run_hll_accuracy(
    output: &Path,
    inputs: &[SketchInput<'static>],
    true_distinct: usize,
    total_items: usize,
) -> Result<(), Box<dyn Error>> {
    let header =
        "implementation,num_workers,run,total_items,true_distinct,estimate,relative_error";
    let file = create_csv(output, header)?;
    let mut w = BufWriter::new(file);

    for &num_workers in &THREAD_COUNTS {
        let parts = partition(inputs, num_workers);
        for run in 1..=RUNS {
            let all_deltas: Vec<Vec<HllDelta>> = std::thread::scope(|s| {
                let handles: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        s.spawn(move || {
                            let mut sketch = HyperLogLog::<Regular>::default();
                            let mut deltas = Vec::new();
                            for input in *part {
                                sketch.insert_emit_delta(input, &mut |d| deltas.push(d));
                            }
                            deltas
                        })
                    })
                    .collect();
                handles.into_iter().map(|h: std::thread::ScopedJoinHandle<'_, Vec<HllDelta>>| h.join().unwrap()).collect()
            });

            let mut agg = HyperLogLog::<Regular>::default();
            for batch in &all_deltas {
                for &delta in batch {
                    agg.apply_delta(delta);
                }
            }

            let estimate = agg.estimate();
            let re = (estimate as f64 - true_distinct as f64).abs() / true_distinct as f64;
            writeln!(
                w,
                "octo_hll,{num_workers},{run},{total_items},{true_distinct},{estimate},{re:.12}",
            )?;
        }
    }
    w.flush()?;
    eprintln!("Wrote {}", output.display());
    Ok(())
}

fn cms_errors(
    sketch: &CountMin<M5x32K, FastPath>,
    heavy_hitters: &[(i64, u64)],
) -> (f64, f64, f64) {
    let mut sum_re = 0.0;
    let mut max_re: f64 = 0.0;
    let mut sum_ae = 0.0;
    for &(key, true_count) in heavy_hitters {
        let est = sketch.estimate(&SketchInput::I64(key)) as f64;
        let ae = (est - true_count as f64).abs();
        let re = ae / true_count as f64;
        sum_re += re;
        if re > max_re {
            max_re = re;
        }
        sum_ae += ae;
    }
    let n = heavy_hitters.len() as f64;
    (sum_re / n, max_re, sum_ae / n)
}

fn cs_errors(
    sketch: &Count<M5x32K, FastPath>,
    heavy_hitters: &[(i64, u64)],
) -> (f64, f64, f64) {
    let mut sum_re = 0.0;
    let mut max_re: f64 = 0.0;
    let mut sum_ae = 0.0;
    for &(key, true_count) in heavy_hitters {
        let est = sketch.estimate(&SketchInput::I64(key));
        let ae = (est - true_count as f64).abs();
        let re = ae / true_count as f64;
        sum_re += re;
        if re > max_re {
            max_re = re;
        }
        sum_ae += ae;
    }
    let n = heavy_hitters.len() as f64;
    (sum_re / n, max_re, sum_ae / n)
}

fn load_baseline(path: &Path) -> Result<(Vec<i64>, HashMap<i64, u64>), Box<dyn Error>> {
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
    let mut freqs = HashMap::new();
    for chunk in buffer.chunks_exact(8) {
        let v = i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        values.push(v);
        *freqs.entry(v).or_insert(0u64) += 1;
    }
    Ok((values, freqs))
}

fn create_csv(path: &Path, header: &str) -> Result<File, Box<dyn Error>> {
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
    w.flush()?;
    Ok(w.into_inner()?)
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_cms = PathBuf::from("../output/octo_accuracy_cms.csv");
    let mut output_cs = PathBuf::from("../output/octo_accuracy_cs.csv");
    let mut output_hll = PathBuf::from("../output/octo_accuracy_hll.csv");

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = PathBuf::from(args.next().ok_or("--data requires a path")?),
            "--output-cms" => {
                output_cms = PathBuf::from(args.next().ok_or("--output-cms requires a path")?)
            }
            "--output-cs" => {
                output_cs = PathBuf::from(args.next().ok_or("--output-cs requires a path")?)
            }
            "--output-hll" => {
                output_hll = PathBuf::from(args.next().ok_or("--output-hll requires a path")?)
            }
            "--help" | "-h" => {
                println!("Usage: octo_accuracy [--data PATH] [--output-cms PATH] [--output-cs PATH] [--output-hll PATH]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output_cms,
        output_cs,
        output_hll,
    })
}
