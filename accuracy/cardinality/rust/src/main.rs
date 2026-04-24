mod baseline;
mod output;
mod seeds;

use baseline::load_baseline;
use datasketches::hll::{HllSketch, HllType};
use output::{write_csv, AccuracyRow};
use seeds::SEEDS;
use sketch_oxide::cardinality::HyperLogLog as OxideHyperLogLog;
use sketch_oxide::Sketch;
use asap_sketchlib::{
    ErtlMLE, HyperLogLogHIPP12, HyperLogLogHIPP14, HyperLogLogHIPP16,
    HyperLogLogP12, HyperLogLogP14, HyperLogLogP16, DataInput,
};
use std::env;
use std::error::Error;
use std::path::PathBuf;

const IMPLEMENTATION_RUST_DATASKETCHES: &str = "rust_datasketches_hll";
const IMPLEMENTATION_RUST_ASAP_ERTLMLE: &str = "rust_asap_sketchlib_hll_ertlmle";
const IMPLEMENTATION_RUST_ASAP_HIP: &str = "rust_asap_sketchlib_hll_hip";
const IMPLEMENTATION_RUST_OXIDE: &str = "rust_sketch_oxide_hll";
const LG_K_LIST: &[u8] = &[12, 14, 16];

#[derive(Debug)]
struct Args {
    data: PathBuf,
    output_summary: PathBuf,
    implementation_filter: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let baseline = load_baseline(&args.data)?;

    let mut rows = Vec::new();
    match args.implementation_filter.as_deref() {
        None => {
            rows.extend(run_asap_ertlmle(&baseline.values, baseline.distinct_items()));
            rows.extend(run_asap_hip(&baseline.values, baseline.distinct_items()));
            rows.extend(run_oxide(&baseline.values, baseline.distinct_items()));
            rows.extend(run_datasketches_rust(
                &baseline.values,
                baseline.distinct_items(),
            ));
        }
        Some(IMPLEMENTATION_RUST_ASAP_ERTLMLE) => {
            rows.extend(run_asap_ertlmle(&baseline.values, baseline.distinct_items()));
        }
        Some(IMPLEMENTATION_RUST_ASAP_HIP) => {
            rows.extend(run_asap_hip(&baseline.values, baseline.distinct_items()));
        }
        Some(IMPLEMENTATION_RUST_OXIDE) => {
            rows.extend(run_oxide(&baseline.values, baseline.distinct_items()));
        }
        Some(IMPLEMENTATION_RUST_DATASKETCHES) => {
            rows.extend(run_datasketches_rust(
                &baseline.values,
                baseline.distinct_items(),
            ));
        }
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected one of \
                 {IMPLEMENTATION_RUST_ASAP_ERTLMLE}, {IMPLEMENTATION_RUST_ASAP_HIP}, \
                 {IMPLEMENTATION_RUST_OXIDE}, {IMPLEMENTATION_RUST_DATASKETCHES}"
            )
            .into());
        }
    }

    write_csv(&args.output_summary, &rows, false)?;
    Ok(())
}

trait SketchInsertEstimate {
    fn sketch_insert(&mut self, input: &DataInput);
    fn sketch_estimate(&self) -> usize;
}

impl SketchInsertEstimate for HyperLogLogP12<ErtlMLE> {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() }
}

impl SketchInsertEstimate for HyperLogLogP14<ErtlMLE> {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() }
}

impl SketchInsertEstimate for HyperLogLogP16<ErtlMLE> {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() }
}

impl SketchInsertEstimate for HyperLogLogHIPP12 {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() as usize }
}

impl SketchInsertEstimate for HyperLogLogHIPP14 {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() as usize }
}

impl SketchInsertEstimate for HyperLogLogHIPP16 {
    fn sketch_insert(&mut self, input: &DataInput) { self.insert(input); }
    fn sketch_estimate(&self) -> usize { self.estimate() as usize }
}

fn run_asap_at_precision<S>(
    implementation: &'static str,
    lg_k: u8,
    registers: usize,
    values: &[i64],
    true_distinct: usize,
) -> Vec<AccuracyRow>
where
    S: Default + SketchInsertEstimate,
{
    let mut rows = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let mut sketch = S::default();
        for &value in values {
            sketch.sketch_insert(&DataInput::U64(seeded_key(value, seed)));
        }
        rows.push(build_row(
            implementation,
            "rust",
            seed,
            lg_k,
            registers,
            values.len(),
            true_distinct,
            sketch.sketch_estimate() as f64,
        ));
    }
    rows
}

fn run_asap_ertlmle(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::new();
    for &lg_k in LG_K_LIST {
        match lg_k {
            12 => rows.extend(run_asap_at_precision::<HyperLogLogP12<ErtlMLE>>(
                IMPLEMENTATION_RUST_ASAP_ERTLMLE, 12, 1 << 12, values, true_distinct,
            )),
            14 => rows.extend(run_asap_at_precision::<HyperLogLogP14<ErtlMLE>>(
                IMPLEMENTATION_RUST_ASAP_ERTLMLE, 14, 1 << 14, values, true_distinct,
            )),
            16 => rows.extend(run_asap_at_precision::<HyperLogLogP16<ErtlMLE>>(
                IMPLEMENTATION_RUST_ASAP_ERTLMLE, 16, 1 << 16, values, true_distinct,
            )),
            _ => unreachable!(),
        }
    }
    rows
}

fn run_asap_hip(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::new();
    for &lg_k in LG_K_LIST {
        match lg_k {
            12 => rows.extend(run_asap_at_precision::<HyperLogLogHIPP12>(
                IMPLEMENTATION_RUST_ASAP_HIP, 12, 1 << 12, values, true_distinct,
            )),
            14 => rows.extend(run_asap_at_precision::<HyperLogLogHIPP14>(
                IMPLEMENTATION_RUST_ASAP_HIP, 14, 1 << 14, values, true_distinct,
            )),
            16 => rows.extend(run_asap_at_precision::<HyperLogLogHIPP16>(
                IMPLEMENTATION_RUST_ASAP_HIP, 16, 1 << 16, values, true_distinct,
            )),
            _ => unreachable!(),
        }
    }
    rows
}

fn run_oxide(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::new();
    for &lg_k in LG_K_LIST {
        let registers = 1usize << lg_k;
        for &seed in &SEEDS {
            let mut sketch = OxideHyperLogLog::new(lg_k).expect("valid precision");
            for &value in values {
                sketch.update(&seeded_key(value, seed));
            }
            rows.push(build_row(
                IMPLEMENTATION_RUST_OXIDE,
                "rust",
                seed,
                lg_k,
                registers,
                values.len(),
                true_distinct,
                sketch.estimate(),
            ));
        }
    }
    rows
}

fn run_datasketches_rust(values: &[i64], true_distinct: usize) -> Vec<AccuracyRow> {
    let mut rows = Vec::new();
    for &lg_k in LG_K_LIST {
        let registers = 1usize << lg_k;
        for &seed in &SEEDS {
            let mut sketch = HllSketch::new(lg_k, HllType::Hll8);
            for &value in values {
                sketch.update(seeded_key(value, seed));
            }
            rows.push(build_row(
                IMPLEMENTATION_RUST_DATASKETCHES,
                "rust",
                seed,
                lg_k,
                registers,
                values.len(),
                true_distinct,
                sketch.estimate(),
            ));
        }
    }
    rows
}

fn build_row(
    implementation: &'static str,
    language: &'static str,
    seed: u64,
    lg_k: u8,
    registers: usize,
    total_items: usize,
    true_distinct: usize,
    estimate: f64,
) -> AccuracyRow {
    let relative_error = (estimate - true_distinct as f64).abs() / true_distinct as f64;
    AccuracyRow {
        implementation,
        language,
        seed,
        lg_k,
        registers,
        total_items,
        true_distinct,
        estimate,
        relative_error,
    }
}

fn seeded_key(value: i64, seed: u64) -> u64 {
    splitmix64((value as u64) ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output_summary = PathBuf::from("../output/hll_accuracy_results_rust.csv");
    let mut implementation_filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                data = PathBuf::from(args.next().ok_or("--data requires a path")?);
            }
            "--output-summary" => {
                output_summary =
                    PathBuf::from(args.next().ok_or("--output-summary requires a path")?);
            }
            "--impl" => {
                implementation_filter = Some(args.next().ok_or("--impl requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release -- [--data PATH] [--output-summary PATH] [--impl NAME]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        data,
        output_summary,
        implementation_filter,
    })
}
