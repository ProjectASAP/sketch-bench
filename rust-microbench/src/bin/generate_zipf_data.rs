use rand::{Rng, SeedableRng, rngs::StdRng};
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const DEFAULT_NUM_VALUES: usize = 10_000_000;
const DEFAULT_ZIPF_EXPONENT: f64 = 1.1;
const DEFAULT_SUPPORT_SIZE: usize = 500_000;
const DEFAULT_SEED: u64 = 42;

#[derive(Debug)]
struct Args {
    num_values: usize,
    zipf_exponent: f64,
    support_size: usize,
    seed: u64,
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let cdf = build_cdf(args.support_size, args.zipf_exponent);
    write_zipf_i64_stream(&args.output, args.num_values, args.seed, &cdf)?;

    println!(
        "generated {} values to {}",
        args.num_values,
        args.output.display()
    );
    println!(
        "zipf_exponent={}, support_size={}, seed={}",
        args.zipf_exponent, args.support_size, args.seed
    );
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut num_values = DEFAULT_NUM_VALUES;
    let mut zipf_exponent = DEFAULT_ZIPF_EXPONENT;
    let mut support_size = DEFAULT_SUPPORT_SIZE;
    let mut seed = DEFAULT_SEED;
    let mut output: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--num-values" => {
                num_values = args
                    .next()
                    .ok_or("--num-values requires a value")?
                    .parse()?;
            }
            "--zipf-exponent" => {
                zipf_exponent = args
                    .next()
                    .ok_or("--zipf-exponent requires a value")?
                    .parse()?;
            }
            "--support-size" => {
                support_size = args
                    .next()
                    .ok_or("--support-size requires a value")?
                    .parse()?;
            }
            "--seed" => {
                seed = args.next().ok_or("--seed requires a value")?.parse()?;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a path")?,
                ));
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --bin generate_zipf_data --release -- \
[--num-values N] [--zipf-exponent S] [--support-size K] [--seed SEED] [--output PATH]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    if num_values == 0 || support_size == 0 || zipf_exponent <= 0.0 {
        return Err(
            "parameters must satisfy: --num-values > 0, --support-size > 0, --zipf-exponent > 0"
                .into(),
        );
    }

    let output =
        output.unwrap_or_else(|| default_output_path(num_values, zipf_exponent, support_size));

    Ok(Args {
        num_values,
        zipf_exponent,
        support_size,
        seed,
        output,
    })
}

fn default_output_path(num_values: usize, zipf_exponent: f64, support_size: usize) -> PathBuf {
    PathBuf::from("../input").join(format!(
        "benchmark_data_{}m_int64_zipf_s{}_k{}.bin",
        num_values / 1_000_000,
        format_exponent_tag(zipf_exponent),
        support_size
    ))
}

fn format_exponent_tag(exponent: f64) -> String {
    let mut tag = exponent.to_string().replace('.', "");
    while tag.ends_with('0') {
        tag.pop();
    }
    if tag.is_empty() { "0".to_string() } else { tag }
}

fn build_cdf(support_size: usize, zipf_exponent: f64) -> Vec<f64> {
    let mut cdf = Vec::with_capacity(support_size);
    let mut normalizer = 0.0;
    for rank in 1..=support_size {
        normalizer += 1.0 / (rank as f64).powf(zipf_exponent);
    }

    let mut cumulative = 0.0;
    for rank in 1..=support_size {
        cumulative += (1.0 / (rank as f64).powf(zipf_exponent)) / normalizer;
        cdf.push(cumulative);
    }
    if let Some(last) = cdf.last_mut() {
        *last = 1.0;
    }
    cdf
}

fn write_zipf_i64_stream(
    output: &Path,
    num_values: usize,
    seed: u64,
    cdf: &[f64],
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = File::create(output)?;
    let mut writer = BufWriter::new(file);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut bytes = [0u8; std::mem::size_of::<i64>()];

    for _ in 0..num_values {
        let sample = rng.random::<f64>();
        let rank = cdf.partition_point(|&cutoff| cutoff < sample) + 1;
        bytes.copy_from_slice(&(rank as i64).to_le_bytes());
        writer.write_all(&bytes)?;
    }

    writer.flush()?;
    Ok(())
}
