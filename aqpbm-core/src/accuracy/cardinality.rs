//! Cardinality-algorithm ground truth (HLL). Exact distinct
//! count from the generated column; reports relative error.

use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{distinct, GroundTruth};

/// How many times to put the one question. A cheap estimator answers in ~1 ns,
/// inside `Instant::now()`'s 20-50 ns noise floor, so one call would report
/// only timer jitter. The count is this statistic's knowledge, not the
/// runner's: it follows from how cheap the answer is.
const QUERY_TIMING_REPEATS: usize = 4096;

#[derive(Debug, Default, Clone, Copy)]
pub struct CardinalityGT {
    pub column: usize,
}

impl GroundTruth for CardinalityGT {
    /// The exact distinct count.
    type Truth = f64;
    /// There is one question, asked repeatedly, so a probe carries nothing.
    type Probe = ();
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<f64, DataGenError> {
        Ok(distinct(table.column(self.column)?) as f64)
    }

    fn probes(&self, _truth: &f64) -> Vec<()> {
        vec![(); QUERY_TIMING_REPEATS]
    }

    fn score(&self, truth: &f64, _probes: &[()], answers: &[f64]) -> BTreeMap<String, f64> {
        // Every answer is to the same question, so the first is the estimate
        // and the rest existed to make the timing readable.
        let est = answers.first().copied().unwrap_or(0.0);
        let abs_err = (est - truth).abs();
        let rel_err = if *truth > 0.0 { abs_err / truth } else { 0.0 };
        // The null estimator answers 0, giving `relative_error = 1.0` exactly.
        // Any implementation scoring above 1.0 is worse than doing no work.
        BTreeMap::from([
            ("truth".to_string(), *truth),
            ("estimate".to_string(), est),
            ("absolute_error".to_string(), abs_err),
            ("relative_error".to_string(), rel_err),
        ])
    }
}
