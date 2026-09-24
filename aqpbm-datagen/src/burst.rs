//! Deterministic interval bursts for AutoSketch-style workload search.

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::DataGenError;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurstSpec {
    pub interval_rows: usize,
    pub burst_intervals: usize,
    /// Additional rows per selected interval, relative to its base size.
    pub extra_fraction: f64,
    pub seed: u64,
}

/// Keep the base stream and inject resampled traffic into randomly selected
/// intervals. Selection and resampling use one seeded RNG, so ERP provenance
/// completely determines the generated workload.
pub fn inject_interval_bursts<T: Clone>(
    input: &[T],
    spec: BurstSpec,
) -> Result<Vec<T>, DataGenError> {
    if spec.interval_rows == 0 || !spec.extra_fraction.is_finite() || spec.extra_fraction < 0.0 {
        return Err(DataGenError::BadParam("invalid burst specification".into()));
    }
    let interval_count = input.len().div_ceil(spec.interval_rows);
    if spec.burst_intervals > interval_count {
        return Err(DataGenError::BadParam(
            "burst_intervals exceeds available intervals".into(),
        ));
    }
    let mut intervals: Vec<_> = (0..interval_count).collect();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(spec.seed);
    for index in (1..intervals.len()).rev() {
        intervals.swap(index, rng.random_range(0..=index));
    }
    intervals.truncate(spec.burst_intervals);
    intervals.sort_unstable();

    let extra_capacity = (input.len() as f64 * spec.extra_fraction).ceil() as usize;
    let mut output = Vec::with_capacity(input.len().saturating_add(extra_capacity));
    for interval in 0..interval_count {
        let start = interval * spec.interval_rows;
        let end = (start + spec.interval_rows).min(input.len());
        let values = &input[start..end];
        output.extend_from_slice(values);
        if intervals.binary_search(&interval).is_ok() && !values.is_empty() {
            let extra = (values.len() as f64 * spec.extra_fraction).round() as usize;
            output.extend((0..extra).map(|_| values[rng.random_range(0..values.len())].clone()));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_are_reproducible_and_preserve_base_traffic() {
        let input: Vec<_> = (0..100).collect();
        let spec = BurstSpec {
            interval_rows: 10,
            burst_intervals: 2,
            extra_fraction: 0.5,
            seed: 7,
        };
        let first = inject_interval_bursts(&input, spec).unwrap();
        assert_eq!(first, inject_interval_bursts(&input, spec).unwrap());
        assert_eq!(first.len(), 110);
        assert!(input.iter().all(|value| first.contains(value)));
    }

    #[test]
    fn impossible_burst_count_fails_loud() {
        assert!(inject_interval_bursts(
            &[1, 2],
            BurstSpec {
                interval_rows: 2,
                burst_intervals: 2,
                extra_fraction: 1.0,
                seed: 1,
            }
        )
        .is_err());
    }
}
