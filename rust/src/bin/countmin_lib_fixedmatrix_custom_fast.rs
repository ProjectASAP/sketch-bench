#[path = "../bench_common.rs"]
mod bench_common;

use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
};
use sketchlib_rust::{CountMin, FastPath, SketchInput, impl_fixed_matrix};

impl_fixed_matrix!(CustomCountMinMatrixI32U128, i32, 5, 65538, u128);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let results = run_benchmark_i64(
        BenchmarkConfig {
            implementation_name: "sketchlib_countmin_fixedmatrix_custom_fastpath",
            data_path: data_path.as_path(),
            warmup_items: DEFAULT_WARMUP_ITEMS,
            measure_items: DEFAULT_MEASURE_ITEMS,
            trials: DEFAULT_TRIALS,
            discard_trials: DEFAULT_DISCARD_TRIALS,
        },
        || {
            CountMin::<CustomCountMinMatrixI32U128, FastPath>::from_storage(
                CustomCountMinMatrixI32U128::default(),
            )
        },
        |sketch, value| sketch.insert(&SketchInput::I64(value)),
    )?;
    print_results(&results);
    Ok(())
}
