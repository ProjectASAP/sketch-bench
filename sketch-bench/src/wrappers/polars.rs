//! Polars-backed implementations, one per family (`hll/polars`,
//! `cms/polars`, `countsketch/polars`, `kll/polars`, `dd/polars`).
//! These are real `--impl` rows, not accuracy baselines: they
//! compute the exact answer through a DataFrame engine, and the
//! point of racing them is throughput. Their exactness is
//! incidental — the accuracy ground truth lives in `accuracy/`.
//!
//! Mirrors the legacy `throughput/polars_{cardinality,freq,
//! quantile}/` binaries: buffer the stream into a `Vec<i64>`,
//! then on `finalize_for_query` build a `DataFrame` once and run
//! the relevant Polars expression. The total work is billed to
//! the insert phase, matching what legacy measured (insert
//! throughput = items / (push + DataFrame + collect)).
//!
//! Per-call `query()` is a cached lookup — so under
//! `--raw-csv --accuracy` the per-call CSV rows report the
//! post-finalize lookup cost (≈ ns), not the heavyweight Polars
//! compute. That's accurate: by the time the comparator queries
//! the polars baseline, the work is already done.

use std::collections::HashMap;

use crate::init::{BuildError, InitSketch};
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;
use polars::prelude::*;

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

impl InitSketch for PolarsCardinality {
    /// Polars computes the exact answer; it has no `(rows, cols)` to tune, so
    /// it ignores the `ParamSet` and builds unconditionally. Under a
    /// multi-point `--config` sweep it therefore repeats the same numbers at
    /// every grid point — a known rough edge, kept until an impl's own
    /// (possibly empty) parameter space is modelled instead of the family's.
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Ok(Self {
            buf: Vec::new(),
            estimate: 0.0,
        })
    }
}

impl Sketch for PolarsCardinality {
    type Item = i64;
    type Query = ();
    type Answer = f64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn finalize_for_query(&mut self) {
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

    fn query(&self, _: ()) -> f64 {
        self.estimate
    }

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

impl Sketch for PolarsFrequencyCms {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
    fn query(&self, q: i64) -> u64 {
        self.0.query(q)
    }
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

impl Sketch for PolarsFrequencyCs {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
    fn query(&self, q: i64) -> u64 {
        self.0.query(q)
    }
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

/// Polars-backed quantile baseline. The heavy work (one polars
/// sort + 101-point quantile grid build) lives in
/// `finalize_for_query`, which the runner now times separately
/// into `RunMetrics::finalize_wall_time_ns`. Insert remains pure
/// `Vec::push`; per-call `query()` is an array lookup.
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

impl Sketch for PolarsQuantileKll {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
    fn query(&self, q: f64) -> f64 {
        self.0.query(q)
    }
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

impl Sketch for PolarsQuantileDd {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
    fn query(&self, q: f64) -> f64 {
        self.0.query(q)
    }
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}
