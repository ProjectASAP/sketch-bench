#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, COLS,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, ROWS,
};
use datasketches::countmin::CountMinSketch;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();

    let config = BenchmarkConfig {
        implementation_name: "datasketches_countmin",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let num_hashes: u8 = ROWS as u8;
    let num_buckets: u32 = COLS as u32;

    let results = run_benchmark_i64(
        config,
        move || CountMinSketch::new(num_hashes, num_buckets),
        |sketch, value| sketch.update(value),
    )?;

    print_results(&results);
    Ok(())
}
