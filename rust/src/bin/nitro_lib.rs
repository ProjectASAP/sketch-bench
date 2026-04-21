#[path = "../bench_common.rs"]
mod bench_common;

use asap_sketchlib::{NitroBatch, Vector2D};
use bench_common::{
    default_data_path, print_results, run_benchmark_i64_batch, BenchmarkConfig, COLS,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
    NITRO_RATE, ROWS,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "sketchlib_nitro_batch",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_i64_batch(
        config,
        || {
            let mut sk = Vector2D::init(ROWS, COLS);
            sk.fill(0_u32);
            NitroBatch::with_target(NITRO_RATE, sk)
        },
        |nitro, values| nitro.insert(values),
    )?;

    print_results(&results);
    Ok(())
}
