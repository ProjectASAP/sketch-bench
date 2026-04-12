#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_strings, BenchmarkConfig,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
    ELASTIC_BUCKETS,
};
use asap_sketchlib::{DefaultXxHasher, Elastic as LibElastic};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "sketchlib_elastic",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_strings(
        config,
        || LibElastic::<DefaultXxHasher>::init_with_length(ELASTIC_BUCKETS as i32),
        |sketch, key| sketch.insert(key.to_owned()),
    )?;

    print_results(&results);
    Ok(())
}
