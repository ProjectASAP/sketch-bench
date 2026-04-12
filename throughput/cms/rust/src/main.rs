use datasketches::countmin::CountMinSketch;
use sketch_oxide::frequency::CountMinSketch as OxideCountMin;
use asap_sketchlib::{
    impl_fixed_matrix, hash_for_matrix_seeded_generic, CountMin, FastPath, HeapItem,
    MatrixHashType, SketchHasher, SketchInput,
};
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use twox_hash::{XxHash3_128, XxHash3_64};

const ROWS: usize = 5;
const COLS: usize = 2048;
const EPSILON: f64 = 0.0013;
const DELTA: f64 = 0.0067;
const SEEDS: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
const IMPLEMENTATION_RUST_DATASKETCHES: &str = "rust_datasketches_cms";
const IMPLEMENTATION_RUST_OXIDE: &str = "rust_oxide_cms";
const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_cms";
const CSV_HEADER: &str =
    "implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec";

impl_fixed_matrix!(M5x2K, i32, 5, 2048);

#[derive(Clone, Debug)]
struct ThroughputRow {
    implementation: &'static str,
    language: &'static str,
    seed: u64,
    rows: usize,
    cols: usize,
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
            self.seed,
            self.rows,
            self.cols,
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
            rows.extend(run_datasketches(&data));
            rows.extend(run_oxide(&data));
            rows.extend(run_sketchlib(&data));
        }
        Some(IMPLEMENTATION_RUST_DATASKETCHES) => rows.extend(run_datasketches(&data)),
        Some(IMPLEMENTATION_RUST_OXIDE) => rows.extend(run_oxide(&data)),
        Some(IMPLEMENTATION_RUST_SKETCHLIB) => rows.extend(run_sketchlib(&data)),
        Some(other) => {
            return Err(format!(
                "unsupported --impl value: {other}; expected \
                 {IMPLEMENTATION_RUST_DATASKETCHES}, {IMPLEMENTATION_RUST_OXIDE}, \
                 or {IMPLEMENTATION_RUST_SKETCHLIB}"
            )
            .into())
        }
    }

    write_rows(&args.output, &rows)?;
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut data = PathBuf::from("../../../input/benchmark_data_10m_int64_zipf_s11_k100000.bin");
    let mut output = PathBuf::from("../output/cms_throughput_results_rust.csv");
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

fn run_datasketches(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let mut sketch = CountMinSketch::with_seed(ROWS as u8, COLS as u32, seed);
        let start = Instant::now();
        for &value in data {
            sketch.update(value);
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_DATASKETCHES,
            language: "rust",
            seed,
            rows: ROWS,
            cols: COLS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn run_oxide(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(SEEDS.len());
    for &seed in &SEEDS {
        let mut sketch = OxideCountMin::new(EPSILON, DELTA).expect("valid CMS parameters");
        let _ = seed;
        let start = Instant::now();
        for &value in data {
            sketch.update(&value);
        }
        std::hint::black_box(&sketch);
        let elapsed = start.elapsed().as_nanos();
        rows.push(ThroughputRow {
            implementation: IMPLEMENTATION_RUST_OXIDE,
            language: "rust",
            seed,
            rows: ROWS,
            cols: COLS,
            total_items: data.len(),
            total_nanoseconds: elapsed,
            throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
        });
    }
    rows
}

fn run_sketchlib(data: &[i64]) -> Vec<ThroughputRow> {
    let mut rows = Vec::with_capacity(SEEDS.len());
    macro_rules! push_seed {
        ($seed:expr, $hasher:ty) => {{
            let mut sketch = CountMin::<M5x2K, FastPath, $hasher>::from_storage(M5x2K::default());
            let start = Instant::now();
            for &value in data {
                sketch.insert(&SketchInput::I64(value));
            }
            std::hint::black_box(&sketch);
            let elapsed = start.elapsed().as_nanos();
            rows.push(ThroughputRow {
                implementation: IMPLEMENTATION_RUST_SKETCHLIB,
                language: "rust",
                seed: $seed,
                rows: ROWS,
                cols: COLS,
                total_items: data.len(),
                total_nanoseconds: elapsed,
                throughput_items_per_sec: data.len() as f64 * 1_000_000_000.0 / elapsed as f64,
            });
        }};
    }

    push_seed!(1, H01);
    push_seed!(2, H02);
    push_seed!(3, H03);
    push_seed!(4, H04);
    push_seed!(5, H05);
    push_seed!(6, H06);
    push_seed!(7, H07);
    push_seed!(8, H08);
    push_seed!(9, H09);
    push_seed!(10, H10);
    rows
}

macro_rules! define_seeded_hasher {
    ($name:ident, $seed:expr) => {
        #[derive(Clone, Debug)]
        struct $name;

        impl SketchHasher for $name {
            type HashType = MatrixHashType;

            fn hash64_seeded(d: usize, key: &SketchInput) -> u64 {
                hash_input64($seed + d as u64, key)
            }

            fn hash128_seeded(d: usize, key: &SketchInput) -> u128 {
                hash_input128($seed + d as u64, key)
            }

            fn hash_item64_seeded(d: usize, key: &HeapItem) -> u64 {
                hash_heap64($seed + d as u64, key)
            }

            fn hash_item128_seeded(d: usize, key: &HeapItem) -> u128 {
                hash_heap128($seed + d as u64, key)
            }

            fn hash_for_matrix_seeded(
                seed_idx: usize,
                rows: usize,
                cols: usize,
                key: &SketchInput,
            ) -> Self::HashType {
                hash_for_matrix_seeded_generic::<Self>(seed_idx, rows, cols, key)
            }
        }
    };
}

define_seeded_hasher!(H01, 1);
define_seeded_hasher!(H02, 2);
define_seeded_hasher!(H03, 3);
define_seeded_hasher!(H04, 4);
define_seeded_hasher!(H05, 5);
define_seeded_hasher!(H06, 6);
define_seeded_hasher!(H07, 7);
define_seeded_hasher!(H08, 8);
define_seeded_hasher!(H09, 9);
define_seeded_hasher!(H10, 10);

#[inline(always)]
fn hash_input64(seed: u64, key: &SketchInput) -> u64 {
    match key {
        SketchInput::I8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::I128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::ISIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::U8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::U128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::USIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::F32(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::F64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::Str(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::String(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::Bytes(v) => XxHash3_64::oneshot_with_seed(seed, v),
    }
}

#[inline(always)]
fn hash_input128(seed: u64, key: &SketchInput) -> u128 {
    match key {
        SketchInput::I8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::I64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::I128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::ISIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        SketchInput::U8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::U64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::U128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        SketchInput::USIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        SketchInput::F32(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::F64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        SketchInput::Str(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::String(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
        SketchInput::Bytes(v) => XxHash3_128::oneshot_with_seed(seed, v),
    }
}

#[inline(always)]
fn hash_heap64(seed: u64, key: &HeapItem) -> u64 {
    match key {
        HeapItem::I8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::I128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::ISIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::U8(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U16(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U32(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::U128(v) => XxHash3_64::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::USIZE(v) => XxHash3_64::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::F32(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::F64(v) => XxHash3_64::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::String(v) => XxHash3_64::oneshot_with_seed(seed, v.as_bytes()),
    }
}

#[inline(always)]
fn hash_heap128(seed: u64, key: &HeapItem) -> u128 {
    match key {
        HeapItem::I8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::I64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::I128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::ISIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as i64).to_ne_bytes()),
        HeapItem::U8(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U16(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U32(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::U64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::U128(v) => XxHash3_128::oneshot_with_seed(seed, &(*v).to_ne_bytes()),
        HeapItem::USIZE(v) => XxHash3_128::oneshot_with_seed(seed, &(*v as u64).to_ne_bytes()),
        HeapItem::F32(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::F64(v) => XxHash3_128::oneshot_with_seed(seed, &v.to_ne_bytes()),
        HeapItem::String(v) => XxHash3_128::oneshot_with_seed(seed, v.as_bytes()),
    }
}
