#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, HLL_PRECISION,
};
use datasketches::hll::{HllSketch, HllType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();

    let config = BenchmarkConfig {
        implementation_name: "datasketches_hll",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let lg_k: u8 = HLL_PRECISION;
    let hll_type = HllType::Hll8;

    let results = run_benchmark_i64(
        config,
        move || HllSketch::new(lg_k, hll_type),
        |sketch, value| sketch.update(value),
    )?;

    print_results(&results);
    Ok(())
}
