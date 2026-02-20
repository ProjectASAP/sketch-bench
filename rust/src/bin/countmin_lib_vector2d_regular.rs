#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, COLS,
    DEFAULT_DISCARD_TRIALS, DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS, ROWS,
};
use sketchlib_rust::{CountMin, RegularPath, SketchInput, Vector2D};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let results = run_benchmark_i64(
        BenchmarkConfig {
            implementation_name: "sketchlib_countmin_vector2d_regularpath",
            data_path: data_path.as_path(),
            warmup_items: DEFAULT_WARMUP_ITEMS,
            measure_items: DEFAULT_MEASURE_ITEMS,
            trials: DEFAULT_TRIALS,
            discard_trials: DEFAULT_DISCARD_TRIALS,
        },
        || CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(ROWS, COLS),
        |sketch, value| sketch.insert(&SketchInput::I64(value)),
    )?;
    print_results(&results);
    Ok(())
}
