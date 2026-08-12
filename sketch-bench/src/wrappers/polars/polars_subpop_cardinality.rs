//! `hydra-hll/polars` — exact subpopulation cardinality over every label subset.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;
use aqpbm_core::workload::Labeled;
use std::collections::HashMap;

use super::fan_out;
use aqpbm_core::accuracy::SubpopCardinalityOps;
use crate::params::HydraHllParams;

/// `hydra-hll/polars` — exact subpopulation cardinality.
#[derive(Default)]
pub struct PolarsSubpopCardinality {
    buf: Vec<Labeled<i64>>,
    distinct: HashMap<String, u64>,
}

impl InitSketch for PolarsSubpopCardinality {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: HydraHllParams = config.parse()?;
        Ok(Self::default())
    }
}

impl Accumulator for PolarsSubpopCardinality {
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
            .group_by([col("g")])
            .agg([col("v").n_unique().alias("c")])
            .collect()
            .expect("polars group_by collect");

        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let counts = result
            .column("c")
            .expect("c column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");

        self.distinct.reserve(groups.len());
        for (g, c) in groups.into_iter().zip(counts) {
            if let (Some(g), Some(c)) = (g, c) {
                self.distinct.insert(g.to_string(), c);
            }
        }
    }
}

impl SubpopCardinalityOps for PolarsSubpopCardinality {
    fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.distinct.get(&labels.join(";")).copied().unwrap_or(0) as f64
    }
}

impl MemoryFootprint for PolarsSubpopCardinality {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<Labeled<i64>>()
            + self.distinct.capacity() * (std::mem::size_of::<String>() + 8)
    }
}

impl BenchImpl for PolarsSubpopCardinality {
    type Params = HydraHllParams;
    const IMPL: &'static str = "polars";
}
