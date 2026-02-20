#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, KLL_K,
};
use sketch_oxide::quantiles::KllSketch as OxideKll;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "oxide_kll",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_i64(
        config,
        || OxideKll::new(KLL_K as u16).expect("valid KLL k"),
        |sketch, value| sketch.update(value as f64),
    )?;

    print_results(&results);
    Ok(())
}
