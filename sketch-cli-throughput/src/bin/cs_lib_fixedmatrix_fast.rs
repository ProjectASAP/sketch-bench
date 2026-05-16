//! Phase 1 probe binary. Same hot loop as
//! `throughput-bench/src/bin/cs_lib_fixedmatrix_fast.rs`, but
//! compiled inside the sketch-bench workspace.

#[path = "../bench_common.rs"]
mod bench_common;

use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix};
use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let results = run_benchmark_i64(
        BenchmarkConfig {
            implementation_name: "sketchlib_throughput_count_fixedmatrix_fastpath",
            data_path: data_path.as_path(),
            warmup_items: DEFAULT_WARMUP_ITEMS,
            measure_items: DEFAULT_MEASURE_ITEMS,
            trials: DEFAULT_TRIALS,
            discard_trials: DEFAULT_DISCARD_TRIALS,
        },
        || Count::<FixedMatrix, FastPath>::default(),
        |sketch, value| sketch.insert(&DataInput::I64(value)),
    )?;
    print_results(&results);
    Ok(())
}
