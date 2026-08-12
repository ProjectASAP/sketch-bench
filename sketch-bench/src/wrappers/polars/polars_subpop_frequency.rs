//! `hydra-cms/polars` — exact subpopulation frequency over every label subset.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;
use aqpbm_core::workload::Labeled;
use std::collections::HashMap;

use super::fan_out;
use aqpbm_core::accuracy::SubpopFrequencyOps;
use crate::params::HydraCmsParams;

/// `hydra-cms/polars` — exact subpopulation frequency.
#[derive(Default)]
pub struct PolarsSubpopFrequency {
    buf: Vec<Labeled<i64>>,
    counts: HashMap<(String, i64), u64>,
}

impl InitSketch for PolarsSubpopFrequency {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: HydraCmsParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsSubpopFrequency {
    type Item = Labeled<i64>;

    #[inline(always)]
    fn update(&mut self, r: &Labeled<i64>) {
        self.buf.push(r.clone());
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
            .group_by([col("g"), col("v")])
            .agg([len().alias("count")])
            .collect()
            .expect("polars group_by collect");

        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let vals = result.column("v").expect("v column");
        let vals = vals.i64().expect("i64 values");
        let counts = result
            .column("count")
            .expect("count column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");

        self.counts.reserve(groups.len());
        for ((g, v), c) in groups.into_iter().zip(vals.into_iter()).zip(counts) {
            if let (Some(g), Some(v), Some(c)) = (g, v, c) {
                self.counts.insert((g.to_string(), v), c);
            }
        }
    }
}

impl SubpopFrequencyOps for PolarsSubpopFrequency {
    type Value = i64;
    fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
        self.counts
            .get(&(labels.join(";"), *value))
            .copied()
            .unwrap_or(0) as f64
    }
}

impl MemoryFootprint for PolarsSubpopFrequency {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<Labeled<i64>>()
            + self.counts.capacity() * (std::mem::size_of::<(String, i64)>() + 8)
    }
}

impl BenchImpl for PolarsSubpopFrequency {
    type Params = HydraCmsParams;
    const IMPL: &'static str = "polars";
}
