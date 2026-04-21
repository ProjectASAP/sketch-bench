#[path = "../bench_common.rs"]
mod bench_common;

use asap_sketchlib::{DataInput, ErtlMLE, HyperLogLog as LibHyperLogLog};
use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "sketchlib_hyperloglog",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_i64(
        config,
        || LibHyperLogLog::<ErtlMLE>::new(),
        |sketch, value| sketch.insert(&DataInput::I64(value)),
    )?;

    print_results(&results);
    Ok(())
}
