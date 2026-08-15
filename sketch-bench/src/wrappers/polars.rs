//! Polars-backed implementations, one per algorithm. Real `--impl` rows, not
//! accuracy baselines: they compute the exact answer through a DataFrame engine
//! and the point of racing them is throughput. They buffer the stream in
//! `update` and build in `prepare`, so race them on
//! `build_throughput_items_per_sec` — the insert column is the `Vec::push`.

use std::collections::HashMap;

use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::{
    CmsParams, CountSketchParams, HllParams, HydraCmsParams, HydraHllParams, HydraKllParams,
    KllParams,
};
use aqpbm_core::config::ParamSet;
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;
use polars::prelude::*;

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

impl InitSketch for PolarsCardinality {
    /// Polars computes the exact answer and has no `(rows, cols)` to tune, so it
    /// ignores the `ParamSet` and builds unconditionally. Its record carries
    /// whatever config the cell was given.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: HllParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            estimate: 0.0,
        })
    }
}


impl MemoryFootprint for PolarsCardinality {
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}


/// Polars-backed frequency: `group_by(v).agg(len)`, then cache
/// the resulting key→count table for O(1) per-key queries.
/// Shared by `cms/polars` and `countsketch/polars`.
#[derive(Default)]
struct PolarsFrequencyCore {
    buf: Vec<i64>,
    counts: HashMap<i64, u64>,
}

impl PolarsFrequencyCore {
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    fn finalize(&mut self) {
        let series = Column::new("v".into(), &self.buf);
        let df = DataFrame::new(vec![series]).expect("DataFrame::new");
        let result = df
            .lazy()
            .group_by([col("v")])
            .agg([len().alias("count")])
            .collect()
            .expect("polars group_by collect");
        let keys = result.column("v").expect("v column");
        let keys = keys.i64().expect("i64 keys");
        let counts = result
            .column("count")
            .expect("count column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");
        self.counts.reserve(keys.len());
        for (k, c) in keys.into_iter().zip(counts.into_iter()) {
            if let (Some(k), Some(c)) = (k, c) {
                self.counts.insert(k, c);
            }
        }
    }
    fn query(&self, q: i64) -> u64 {
        self.counts.get(&q).copied().unwrap_or(0)
    }
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
            + self.counts.capacity() * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>())
    }
}

/// `cms/polars` view of [`PolarsFrequencyCore`].
#[derive(Default)]
pub struct PolarsFrequencyCms(PolarsFrequencyCore);

impl InitSketch for PolarsFrequencyCms {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CmsParams = config.parse()?;
        Ok(Self::default())
    }
}


impl MemoryFootprint for PolarsFrequencyCms {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

/// `countsketch/polars` view of [`PolarsFrequencyCore`].
#[derive(Default)]
pub struct PolarsFrequencyCs(PolarsFrequencyCore);

impl InitSketch for PolarsFrequencyCs {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: CountSketchParams = config.parse()?;
        Ok(Self::default())
    }
}


impl MemoryFootprint for PolarsFrequencyCs {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

/// Polars-backed quantile baseline. The heavy work — one sort plus a 101-point
/// quantile grid — lives in `prepare`, which the runner times separately.
/// Insert is pure `Vec::push`; per-call `query()` is an array lookup.
struct PolarsQuantileCore {
    buf: Vec<i64>,
    quantiles: [f64; 101],
}

impl Default for PolarsQuantileCore {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            quantiles: [0.0; 101],
        }
    }
}

impl PolarsQuantileCore {
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    fn finalize(&mut self) {
        let series = Column::new("v".into(), &self.buf);
        let df = DataFrame::new(vec![series]).expect("DataFrame::new");
        let exprs: Vec<Expr> = (0..=100)
            .map(|i| {
                let p = i as f64 / 100.0;
                col("v")
                    .quantile(lit(p), QuantileMethod::Linear)
                    .alias(format!("q{i}"))
            })
            .collect();
        let result = df
            .lazy()
            .select(exprs)
            .collect()
            .expect("polars quantile collect");
        for i in 0..=100 {
            let c = result
                .column(&format!("q{i}"))
                .expect("quantile column")
                .cast(&DataType::Float64)
                .expect("cast to f64");
            self.quantiles[i] = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
        }
    }
    fn query(&self, p: f64) -> f64 {
        let i = (p * 100.0).round().clamp(0.0, 100.0) as usize;
        self.quantiles[i]
    }
    fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

/// `kll/polars` view of [`PolarsQuantileCore`].
#[derive(Default)]
pub struct PolarsQuantileKll(PolarsQuantileCore);

impl InitSketch for PolarsQuantileKll {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        // Exact, so no knob here does anything. The config is still parsed
        // and discarded: this row is the baseline its sketch siblings are
        // scored against, and a config they refuse must not quietly produce
        // a number here.
        let _p: KllParams = config.parse()?;
        Ok(Self::default())
    }
}


impl MemoryFootprint for PolarsQuantileKll {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

// ---------- statistic membership ----------
// The exact baselines answer the same statistics as the sketches beside them,
// which is what makes their ~0 error a check on the comparator.

impl PolarsCardinality {
    pub fn estimate_distinct(&self) -> f64 {
        self.estimate
    }
}

impl PolarsFrequencyCms {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl PolarsFrequencyCs {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl PolarsQuantileKll {
    pub fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

// ---------- catalog identity ----------
// Each exact baseline is named `polars` inside whichever algorithm its params
// type places it in. A baseline sits on the family's **base** algorithm, so it
// is the one exact answer every structural variant of that family is scored
// against, and the algorithm falls out of `Params`.
//
// The quantile baseline is the exception: `kll` has no base algorithm, since
// every KLL row states a query path. This one arranges its answer once in
// `prepare` and reads it back, which is what `kll-cdf` names, so that is where
// it belongs.

impl BenchImpl for PolarsCardinality { type Params = HllParams; const IMPL: &'static str = "polars"; const SUPPORTS_PREPARE: bool = true; }
impl BenchImpl for PolarsFrequencyCms { type Params = CmsParams; const IMPL: &'static str = "polars"; const SUPPORTS_PREPARE: bool = true; }
impl BenchImpl for PolarsFrequencyCs { type Params = CountSketchParams; const IMPL: &'static str = "polars"; const SUPPORTS_PREPARE: bool = true; }
impl BenchImpl for PolarsQuantileKll {
    type Params = KllParams;
    const ALGORITHM: &'static str = "kll-cdf";
    const IMPL: &'static str = "polars";
    const SUPPORTS_PREPARE: bool = true;
}
// ---------- the grouped baselines ----------
//
// One per Hydra algorithm. These differ from the baselines above in what they
// have to reproduce: a grouped sketch writes each record into every non-empty
// subset of its labels, so a baseline that grouped by one column would be
// answering an easier question and its throughput would not be comparable.
// The fan-out happens in `prepare`, which is where every polars row does its
// work; the insert column stays a `Vec::push`.
//
// # Their error should be zero, and if it is not that is a finding
//
// A depth-1 subset key is the bare label string, with nothing naming the column
// it came from. These baselines reproduce that key space exactly, so when two
// label columns share a value their groups collide here too. The comparator's
// ground truth does not model that collision, so a non-zero error on one of
// these rows is the aliasing #74 records and not an approximation.

/// The `;`-joined key of one label subset, in column order. This is the format
/// `Hydra::update` builds internally, reproduced so the baseline and the sketch
/// answer to the same key.
fn subset_key(parts: &[&str], mask: usize) -> String {
    let mut out = String::new();
    for (j, part) in parts.iter().enumerate() {
        if (mask >> j) & 1 == 1 {
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(part);
        }
    }
    out
}

/// Expand one record into `(subset_key, value)` rows, one per non-empty subset
/// of its labels. Empty label parts are dropped, matching the library's
/// `split(';').filter(|s| !s.is_empty())`.
fn fan_out<V: Copy>(key: &str, value: V, keys: &mut Vec<String>, values: &mut Vec<V>) {
    let parts: Vec<&str> = key.split(';').filter(|s| !s.is_empty()).collect();
    for mask in 1..(1usize << parts.len()) {
        keys.push(subset_key(&parts, mask));
        values.push(value);
    }
}

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

// Exactness is no excuse for accepting a config the rest of the algorithm
// rejects — the rule the `init` bodies above follow. The test that pinned it
// lived on the top-k baseline and went with it; restore one here alongside the
// next row whose config is load-bearing.

// ---------- how this sketch is driven ----------
//
// One function per operation, per sketch. These used to be an
// `impl Accumulator for X` block, which fixed one signature for every
// implementation in the repo. As free functions each states its own
// terms, and `catalog` names them in the row's `SketchOps`.
    #[inline(always)]
pub fn insert_polars_cardinality(sketch: &mut PolarsCardinality, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_polars_cardinality(sketch: &mut PolarsCardinality)
{
        let series = Column::new("v".into(), &sketch.buf);
        let df = DataFrame::new(vec![series]).expect("DataFrame::new");
        let result = df
            .lazy()
            .select([col("v").n_unique().alias("c")])
            .collect()
            .expect("polars n_unique collect");
        let c = result
            .column("c")
            .expect("c column")
            .cast(&DataType::Float64)
            .expect("cast to f64");
        sketch.estimate = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
}
    #[inline(always)]
pub fn insert_polars_frequency_cms(sketch: &mut PolarsFrequencyCms, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_frequency_cms(sketch: &mut PolarsFrequencyCms)
{
        sketch.0.finalize();
}
    #[inline(always)]
pub fn insert_polars_frequency_cs(sketch: &mut PolarsFrequencyCs, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_frequency_cs(sketch: &mut PolarsFrequencyCs)
{
        sketch.0.finalize();
}
    #[inline(always)]
pub fn insert_polars_quantile_kll(sketch: &mut PolarsQuantileKll, v: &i64)
{
        sketch.0.update(v);
}

pub fn prepare_polars_quantile_kll(sketch: &mut PolarsQuantileKll)
{
        sketch.0.finalize();
}
    #[inline(always)]
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
    #[inline(always)]
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
    #[inline(always)]
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

// ---------- the rows this file provides ----------
//
// The exact baselines. Each buffers in `insert` and builds in `prepare`, so
// they all supply a `prepare` and none supplies a `merge`.

use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::RankErrorGT;
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};

pub const CARDINALITY_OPS: SketchOps<PolarsCardinality, i64, (), f64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_cardinality),
    ask: |s, _| s.estimate_distinct(),
        _item: std::marker::PhantomData,
};
pub const FREQUENCY_CMS_OPS: SketchOps<PolarsFrequencyCms, i64, i64, u64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_frequency_cms),
    ask: |s, k| s.estimate_frequency(k),
        _item: std::marker::PhantomData,
};
pub const FREQUENCY_CS_OPS: SketchOps<PolarsFrequencyCs, i64, i64, u64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_frequency_cs),
    ask: |s, k| s.estimate_frequency(k),
        _item: std::marker::PhantomData,
};
pub const QUANTILE_KLL_OPS: SketchOps<PolarsQuantileKll, i64, f64, f64> = SketchOps {
    merge: None,
    prepare: Some(prepare_polars_quantile_kll),
    ask: |s, phi| s.estimate_quantile(*phi),
        _item: std::marker::PhantomData,
};
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


pub fn run_cardinality(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsCardinality, i64, CardinalityGT, _>(
        cfg, data, params, width, insert_polars_cardinality,
        &CARDINALITY_OPS,
    )
}
pub fn run_frequency_cms(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsFrequencyCms, i64, FrequencyGT, _>(
        cfg, data, params, width, insert_polars_frequency_cms,
        &FREQUENCY_CMS_OPS,
    )
}
pub fn run_frequency_cs(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsFrequencyCs, i64, FrequencyGT, _>(
        cfg, data, params, width, insert_polars_frequency_cs,
        &FREQUENCY_CS_OPS,
    )
}
pub fn run_quantile_kll(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<PolarsQuantileKll, i64, RankErrorGT, _>(
        cfg, data, params, width, insert_polars_quantile_kll,
        &QUANTILE_KLL_OPS,
    )
}
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
