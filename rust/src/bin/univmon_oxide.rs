#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_bytes, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, DELTA, EPSILON,
    UNIVMON_MAX_STREAM,
};
use sketch_oxide::universal::UnivMon as OxideUnivMon;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "oxide_univmon",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_bytes(
        config,
        || OxideUnivMon::new(UNIVMON_MAX_STREAM, EPSILON, DELTA).expect("valid UnivMon params"),
        |sketch, key| sketch.update(key, 1.0).expect("UnivMon update succeeds"),
    )?;

    print_results(&results);
    Ok(())
}
