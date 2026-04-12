#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_strings, BenchmarkConfig, COLS,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, ROWS,
    UNIVMON_LAYERS, UNIVMON_MAX_STREAM,
};
use asap_sketchlib::{SketchInput, UnivMon as LibUnivMon};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let config = BenchmarkConfig {
        implementation_name: "sketchlib_univmon",
        data_path: data_path.as_path(),
        warmup_items: DEFAULT_WARMUP_ITEMS,
        measure_items: DEFAULT_MEASURE_ITEMS,
        trials: DEFAULT_TRIALS,
        discard_trials: DEFAULT_DISCARD_TRIALS,
    };

    let results = run_benchmark_strings(
        config,
        || LibUnivMon::init_univmon(UNIVMON_MAX_STREAM as usize, ROWS, COLS, UNIVMON_LAYERS),
        |sketch, key| sketch.fast_insert(&SketchInput::Str(key), 1),
    )?;

    print_results(&results);
    Ok(())
}
