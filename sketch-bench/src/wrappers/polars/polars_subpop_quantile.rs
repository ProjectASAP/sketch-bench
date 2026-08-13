//! `hydra-kll/polars` — exact subpopulation quantile over every label subset.

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;
use aqpbm_core::workload::Labeled;
use std::collections::HashMap;

use super::fan_out;
use aqpbm_core::accuracy::SubpopQuantileOps;
use crate::params::HydraKllParams;

/// `hydra-kll/polars` — exact subpopulation quantile.
#[derive(Default)]
pub struct PolarsSubpopQuantile {
    buf: Vec<Labeled<f64>>,
    sorted: HashMap<String, Vec<f64>>,
}

impl InitSketch for PolarsSubpopQuantile {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: HydraKllParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsSubpopQuantile {
    type Item = Labeled<f64>;

    #[inline(always)]
    fn update(&mut self, r: &Labeled<f64>) {
        self.buf.push(r.clone());
    }

    /// Sorted by `(group, value)` in one pass and then split on the group
    /// boundary, which is a DataFrame sort rather than a per-group one: the
    /// grouped aggregation would hand back a list column this then has to
    /// unnest, and the ordered answer needs the values anyway.
    /// The exact control for the merge square. The runner fills shards with
    /// `update` only and calls `prepare` after the fold, so concatenating the
    /// buffers is the whole of it — the DataFrame work then runs over the whole
    /// stream. Lossless by construction, which is what makes it the control:
    /// any gap on the sketch beside it is the sketch's.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.buf.extend_from_slice(&other.buf);
        Ok(())
    }

    fn prepare(&mut self) {
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &self.buf {
            fan_out(&r.key, r.value, &mut keys, &mut values);
        }
        if keys.is_empty() {
            return;
        }
        let df = DataFrame::new(vec![
            Column::new("g".into(), &keys),
            Column::new("v".into(), &values),
        ])
        .expect("DataFrame::new");
        let result = df
            .lazy()
            .sort(["g", "v"], Default::default())
            .collect()
            .expect("polars sort collect");

        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let vals = result.column("v").expect("v column");
        let vals = vals.f64().expect("f64 values");

        for (g, v) in groups.into_iter().zip(vals) {
            if let (Some(g), Some(v)) = (g, v) {
                // Already ascending within a group, so push keeps it sorted.
                self.sorted.entry(g.to_string()).or_default().push(v);
            }
        }
    }
}

impl SubpopQuantileOps for PolarsSubpopQuantile {
    /// `floor(phi * n)`, clamped to the last index. That is the index whose
    /// rank interval contains `phi * n`, which is what the rank-error
    /// comparator scores against, so an exact answer scores zero.
    fn estimate_subpop_quantile(&self, labels: &[&str], phi: f64) -> f64 {
        let Some(values) = self.sorted.get(&labels.join(";")) else {
            return f64::NAN;
        };
        if values.is_empty() {
            return f64::NAN;
        }
        let idx = ((phi * values.len() as f64).floor() as usize).min(values.len() - 1);
        values[idx]
    }
}

impl MemoryFootprint for PolarsSubpopQuantile {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<Labeled<f64>>()
            + self
                .sorted
                .values()
                .map(|v| v.capacity() * std::mem::size_of::<f64>())
                .sum::<usize>()
    }
}

impl BenchImpl for PolarsSubpopQuantile {
    type Params = HydraKllParams;
    const IMPL: &'static str = "polars";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, value: f64) -> Labeled<f64> {
        Labeled { key: key.to_string(), value }
    }

    fn built() -> PolarsSubpopQuantile {
        PolarsSubpopQuantile::init(&ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 200,
        }))
        .expect("canonical dimensions build")
    }

    /// The ordered statistic is over the union, so the fold has to interleave
    /// the two shards rather than concatenate their sorted runs. `prepare`
    /// sorts after the merge, which is what makes that true.
    #[test]
    fn merging_shards_reorders_across_both() {
        let left_items: Vec<Labeled<f64>> =
            (1..=50).map(|v| record("a;x", v as f64 * 2.0)).collect();
        let right_items: Vec<Labeled<f64>> =
            (1..=50).map(|v| record("a;x", v as f64 * 2.0 - 1.0)).collect();

        let (mut left, mut right) = (built(), built());
        for r in &left_items {
            left.update(r);
        }
        for r in &right_items {
            right.update(r);
        }
        left.merge(&right).expect("the exact control merges");
        left.prepare();

        let mut whole = built();
        for r in left_items.iter().chain(right_items.iter()) {
            whole.update(r);
        }
        whole.prepare();

        for phi in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(
                left.estimate_subpop_quantile(&["a"], phi),
                whole.estimate_subpop_quantile(&["a"], phi),
                "merged and whole-stream disagree at phi={phi}"
            );
        }
        // The union is 1..=100, so the extremes come from opposite shards.
        assert_eq!(left.estimate_subpop_quantile(&["a"], 0.0), 1.0);
        assert_eq!(left.estimate_subpop_quantile(&["a"], 1.0), 100.0);
    }
}
