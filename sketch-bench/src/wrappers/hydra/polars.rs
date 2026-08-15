//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hydra/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use aqpbm_core::workload::Labeled;
use std::collections::HashMap;
use ::polars::prelude::*;
use crate::wrappers::polars_shared::*;

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

impl PolarsSubpopFrequency {
    pub fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
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
    const SUPPORTS_PREPARE: bool = true;
}

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

impl PolarsSubpopCardinality {
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
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
    const SUPPORTS_PREPARE: bool = true;
}

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

impl PolarsSubpopQuantile {
    /// `floor(phi * n)`, clamped to the last index. That is the index whose
    /// rank interval contains `phi * n`, which is what the rank-error
    /// comparator scores against, so an exact answer scores zero.
    pub fn estimate_subpop_quantile(&self, labels: &[&str], phi: f64) -> f64 {
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
    const SUPPORTS_PREPARE: bool = true;
}

pub fn insert_polars_subpop_frequency(sketch: &mut PolarsSubpopFrequency, r: &Labeled<i64>)
{
        sketch.buf.push(r.clone());
}

pub fn prepare_polars_subpop_frequency(sketch: &mut PolarsSubpopFrequency)
{
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &sketch.buf {
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

        sketch.counts.reserve(groups.len());
        for ((g, v), c) in groups.into_iter().zip(vals.into_iter()).zip(counts) {
            if let (Some(g), Some(v), Some(c)) = (g, v, c) {
                sketch.counts.insert((g.to_string(), v), c);
            }
        }
}

pub fn insert_polars_subpop_cardinality(sketch: &mut PolarsSubpopCardinality, r: &Labeled<i64>)
{
        sketch.buf.push(r.clone());
}

pub fn prepare_polars_subpop_cardinality(sketch: &mut PolarsSubpopCardinality)
{
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &sketch.buf {
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

        sketch.distinct.reserve(groups.len());
        for (g, c) in groups.into_iter().zip(counts) {
            if let (Some(g), Some(c)) = (g, c) {
                sketch.distinct.insert(g.to_string(), c);
            }
        }
}

pub fn insert_polars_subpop_quantile(sketch: &mut PolarsSubpopQuantile, r: &Labeled<f64>)
{
        sketch.buf.push(r.clone());
}

pub fn prepare_polars_subpop_quantile(sketch: &mut PolarsSubpopQuantile)
{
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &sketch.buf {
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
                sketch.sorted.entry(g.to_string()).or_default().push(v);
            }
        }
}

pub const SUBPOP_FREQUENCY_OPS: SketchOps<PolarsSubpopFrequency, Labeled<i64>, (String, i64), f64> =
    SketchOps {
        merge: None,
        prepare: Some(prepare_polars_subpop_frequency),
        ask: |s, p| s.estimate_subpop_frequency(&[p.0.as_str()], &p.1),
            _item: std::marker::PhantomData,
    };

pub const SUBPOP_CARDINALITY_OPS: SketchOps<PolarsSubpopCardinality, Labeled<i64>, String, f64> =
    SketchOps {
        merge: None,
        prepare: Some(prepare_polars_subpop_cardinality),
        ask: |s, p| s.estimate_subpop_cardinality(&[p.as_str()]),
            _item: std::marker::PhantomData,
    };

pub const SUBPOP_QUANTILE_OPS: SketchOps<PolarsSubpopQuantile, Labeled<f64>, (String, f64), f64> =
    SketchOps {
        merge: None,
        prepare: Some(prepare_polars_subpop_quantile),
        ask: |s, p| s.estimate_subpop_quantile(&[p.0.as_str()], p.1),
            _item: std::marker::PhantomData,
    };

pub fn run_subpop_frequency(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsSubpopFrequency, Labeled<i64>, SubpopFrequencyGT, _>(
        cfg, data, params, width, insert_polars_subpop_frequency,
        &SUBPOP_FREQUENCY_OPS,
    )
}

pub fn run_subpop_cardinality(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsSubpopCardinality, Labeled<i64>, SubpopCardinalityGT, _>(
        cfg, data, params, width, insert_polars_subpop_cardinality,
        &SUBPOP_CARDINALITY_OPS,
    )
}

pub fn run_subpop_quantile(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsSubpopQuantile, Labeled<f64>, SubpopRankErrorGT, _>(
        cfg, data, params, width, insert_polars_subpop_quantile,
        &SUBPOP_QUANTILE_OPS,
    )
}


