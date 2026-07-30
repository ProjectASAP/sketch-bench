//! Polars-backed implementations, one per algorithm. Real `--impl` rows, not
//! accuracy baselines: they compute the exact answer through a DataFrame engine
//! and the point of racing them is throughput. They buffer the stream in
//! `update` and build in `prepare`, so race them on
//! `build_throughput_items_per_sec` — the insert column is the `Vec::push`.

use std::collections::HashMap;

use aqpbm_core::accuracy::{
    CardinalityOps, FrequencyOps, QuantileOps, SubpopCardinalityOps, SubpopFrequencyOps,
    SubpopQuantileOps, TopKOps,
};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::{
    CmsParams, CountSketchParams, DdParams, HllParams, HydraCmsParams, HydraHllParams,
    HydraKllParams, KllParams, TopkParams,
};
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
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
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self {
            buf: Vec::new(),
            estimate: 0.0,
        })
    }
}

impl Accumulator for PolarsCardinality {
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn prepare(&mut self) {
        let series = Column::new("v".into(), &self.buf);
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
        self.estimate = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
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
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self::default())
    }
}

impl Accumulator for PolarsFrequencyCms {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
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
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self::default())
    }
}

impl Accumulator for PolarsFrequencyCs {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
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
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self::default())
    }
}

impl Accumulator for PolarsQuantileKll {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsQuantileKll {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

/// `dd/polars` view of [`PolarsQuantileCore`].
#[derive(Default)]
pub struct PolarsQuantileDd(PolarsQuantileCore);

impl InitSketch for PolarsQuantileDd {
    /// See [`PolarsCardinality::init`] — no tunable shape, ignores config.
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self::default())
    }
}

impl Accumulator for PolarsQuantileDd {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsQuantileDd {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}


// ---------- statistic membership ----------
// The exact baselines answer the same statistics as the sketches beside them,
// which is what makes their ~0 error a check on the comparator.

impl CardinalityOps for PolarsCardinality {
    fn estimate_distinct(&self) -> f64 {
        self.estimate
    }
}

impl FrequencyOps for PolarsFrequencyCms {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl FrequencyOps for PolarsFrequencyCs {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.query(*key)
    }
}

impl QuantileOps for PolarsQuantileKll {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

impl QuantileOps for PolarsQuantileDd {
    fn estimate_quantile(&self, phi: f64) -> f64 {
        self.0.query(phi)
    }
}

// ---------- catalog identity ----------
// Each exact baseline is named `polars` inside whichever algorithm its params type
// places it in — the algorithm falls out of `Params`.

impl BenchImpl for PolarsCardinality { type Params = HllParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsFrequencyCms { type Params = CmsParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsFrequencyCs { type Params = CountSketchParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsQuantileKll { type Params = KllParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsQuantileDd { type Params = DdParams; const IMPL: &'static str = "polars"; }

/// `topk/polars` — the exact top-k baseline, reusing the frequency baseline's
/// group_by then sorting. Its score checks the comparator: an exact answer must
/// come back at precision = recall = 1.0.
#[derive(Default)]
pub struct PolarsTopK(PolarsFrequencyCore);

impl InitSketch for PolarsTopK {
    /// The one polars baseline that reads its config: `k` is the prefix the
    /// comparator scores against, not a knob to shrug off, so an unreadable `k`
    /// must fail here rather than be silently scored at some other `k`.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: TopkParams = config.parse()?;
        if p.k == 0 {
            return Err(BuildError("topk needs k >= 1".into()));
        }
        Ok(Self::default())
    }
}

impl Accumulator for PolarsTopK {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    /// All of the cost is here, not in `update` — the same split the other
    /// polars baselines use, so the insert column stays a plain `Vec::push`.
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsTopK {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl TopKOps for PolarsTopK {
    type Key = i64;
    fn estimate_topk(&self, k: usize) -> Vec<(i64, u64)> {
        let mut out: Vec<(i64, u64)> = self.0.counts.iter().map(|(&k, &c)| (k, c)).collect();
        out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
    }
}

impl BenchImpl for PolarsTopK {
    type Params = TopkParams;
    const IMPL: &'static str = "polars";
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
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
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

/// `hydra-hll/polars` — exact subpopulation cardinality.
#[derive(Default)]
pub struct PolarsSubpopCardinality {
    buf: Vec<Labeled<i64>>,
    distinct: HashMap<String, u64>,
}

impl InitSketch for PolarsSubpopCardinality {
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
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

/// `hydra-kll/polars` — exact subpopulation quantile.
#[derive(Default)]
pub struct PolarsSubpopQuantile {
    buf: Vec<Labeled<f64>>,
    sorted: HashMap<String, Vec<f64>>,
}

impl InitSketch for PolarsSubpopQuantile {
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn topk_config(params: serde_json::Value) -> ParamSet {
        ParamSet {
            algorithm: "topk".to_string(),
            params,
        }
    }

    /// Exactness is no excuse for accepting a config the rest of the algorithm
    /// rejects: this row is scored at `k`, so an unreadable `k` is a build
    /// failure, not a default.
    #[test]
    fn a_k_that_cannot_be_read_is_a_build_error() {
        let err = match PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "kk": 5 }),
        )) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a misspelled `k` must not build"),
        };
        assert!(err.contains("kk"), "error should name the bad key: {err}");

        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048 })
        ))
        .is_err());
        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "k": 0 })
        ))
        .is_err());
        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "k": 5 })
        ))
        .is_ok());
    }
}
