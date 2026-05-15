// Apples-to-apples test: same loop shape as throughput-bench's
// cs_lib_fixedmatrix_fast.rs, but call into a wrapper *newtype*
// to confirm whether tuple-wrapping `Count<...>` costs anything.

#[path = "../bench_common.rs"]
mod bench_common;

use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix};
use bench_common::{
    default_data_path, print_results, run_benchmark_i64, BenchmarkConfig, DEFAULT_DISCARD_TRIALS,
    DEFAULT_MEASURE_ITEMS, DEFAULT_TRIALS, DEFAULT_WARMUP_ITEMS,
};

// Mirror sketch-cli's wrapper layout.
pub struct CsLibFixedmatrixFast(pub Count<FixedMatrix, FastPath>);

impl CsLibFixedmatrixFast {
    #[inline]
    pub fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = default_data_path();
    let results = run_benchmark_i64(
        BenchmarkConfig {
            implementation_name: "wrapper_count_fixedmatrix_fastpath",
            data_path: data_path.as_path(),
            warmup_items: DEFAULT_WARMUP_ITEMS,
            measure_items: DEFAULT_MEASURE_ITEMS,
            trials: DEFAULT_TRIALS,
            discard_trials: DEFAULT_DISCARD_TRIALS,
        },
        || CsLibFixedmatrixFast(Count::<FixedMatrix, FastPath>::default()),
        // Match sketch-cli's wrapper signature: update takes &i64.
        |sketch, value| sketch.update(&value),
    )?;
    print_results(&results);
    Ok(())
}
