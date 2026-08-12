//! `hydra-kll/polars` — exact subpopulation quantile over every label subset.

use aqpbm_core::accumulator::Accumulator;
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
