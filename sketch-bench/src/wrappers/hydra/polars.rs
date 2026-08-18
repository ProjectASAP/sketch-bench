//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hydra/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::polars_shared::*;
use ::polars::prelude::*;
use aqpbm_core::input_dataset::Labeled;
use aqpbm_core::RunError;
use std::collections::HashMap;

#[derive(Default)]
pub struct PolarsSubpopFrequency {
    buf: Vec<Labeled<i64>>,
    counts: HashMap<(String, i64), u64>,
}

pub fn build_polars_subpop_frequency(config: &ParamSet) -> Result<PolarsSubpopFrequency, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraCmsParams = config.parse()?;
    Ok(PolarsSubpopFrequency::default())
}

impl PolarsSubpopFrequency {
    pub fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
        self.counts
            .get(&(labels.join(";"), *value))
            .copied()
            .unwrap_or(0) as f64
    }
}

pub fn memory_polars_subpop_frequency(sketch: &PolarsSubpopFrequency) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<Labeled<i64>>()
        + sketch.counts.capacity() * (std::mem::size_of::<(String, i64)>() + 8)
}

/// `hydra-hll/polars` — exact subpopulation cardinality.
#[derive(Default)]
pub struct PolarsSubpopCardinality {
    buf: Vec<Labeled<i64>>,
    distinct: HashMap<String, u64>,
}

pub fn build_polars_subpop_cardinality(
    config: &ParamSet,
) -> Result<PolarsSubpopCardinality, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraHllParams = config.parse()?;
    Ok(PolarsSubpopCardinality::default())
}

impl PolarsSubpopCardinality {
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.distinct.get(&labels.join(";")).copied().unwrap_or(0) as f64
    }
}

pub fn memory_polars_subpop_cardinality(sketch: &PolarsSubpopCardinality) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<Labeled<i64>>()
        + sketch.distinct.capacity() * (std::mem::size_of::<String>() + 8)
}

/// `hydra-kll/polars` — exact subpopulation quantile.
#[derive(Default)]
pub struct PolarsSubpopQuantile {
    buf: Vec<Labeled<f64>>,
    sorted: HashMap<String, Vec<f64>>,
}

pub fn build_polars_subpop_quantile(config: &ParamSet) -> Result<PolarsSubpopQuantile, RunError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraKllParams = config.parse()?;
    Ok(PolarsSubpopQuantile::default())
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

pub fn memory_polars_subpop_quantile(sketch: &PolarsSubpopQuantile) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<Labeled<f64>>()
        + sketch
            .sorted
            .values()
            .map(|v| v.capacity() * std::mem::size_of::<f64>())
            .sum::<usize>()
}

pub fn insert_polars_subpop_frequency(sketch: &mut PolarsSubpopFrequency, r: &Labeled<i64>) {
    sketch.buf.push(r.clone());
}

pub fn query_polars_subpop_frequency(
    sketch: &mut PolarsSubpopFrequency,
    probe: &(Vec<String>, i64),
) -> f64 {
    sketch.estimate_subpop_frequency(&labels(&probe.0), &probe.1)
}

pub fn prepare_polars_subpop_frequency(sketch: &mut PolarsSubpopFrequency) {
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
    for ((g, v), c) in groups.into_iter().zip(vals).zip(counts) {
        if let (Some(g), Some(v), Some(c)) = (g, v, c) {
            sketch.counts.insert((g.to_string(), v), c);
        }
    }
}

pub fn insert_polars_subpop_cardinality(sketch: &mut PolarsSubpopCardinality, r: &Labeled<i64>) {
    sketch.buf.push(r.clone());
}

#[allow(clippy::ptr_arg)]
pub fn query_polars_subpop_cardinality(
    sketch: &mut PolarsSubpopCardinality,
    probe: &Vec<String>,
) -> f64 {
    sketch.estimate_subpop_cardinality(&labels(probe))
}

pub fn prepare_polars_subpop_cardinality(sketch: &mut PolarsSubpopCardinality) {
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

pub fn insert_polars_subpop_quantile(sketch: &mut PolarsSubpopQuantile, r: &Labeled<f64>) {
    sketch.buf.push(r.clone());
}

pub fn query_polars_subpop_quantile(
    sketch: &mut PolarsSubpopQuantile,
    probe: &(Vec<String>, f64),
) -> f64 {
    sketch.estimate_subpop_quantile(&labels(&probe.0), probe.1)
}

pub fn prepare_polars_subpop_quantile(sketch: &mut PolarsSubpopQuantile) {
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
