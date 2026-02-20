#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_bytes, BenchmarkConfig,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
    ELASTIC_BUCKETS, ELASTIC_DEPTH,
};
use sketch_oxide::frequency::ElasticSketch as OxideElastic;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "oxide_elasticsketch",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_bytes(
        config,
        || OxideElastic::new(ELASTIC_BUCKETS, ELASTIC_DEPTH).expect("valid Elastic parameters"),
        |sketch, key| sketch.update(key, 1),
    )?;

    print_results(&results);
    Ok(())
}
