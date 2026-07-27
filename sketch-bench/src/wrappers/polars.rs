//! Polars-backed implementations, one per family (`hll/polars`, `cms/polars`,
//! `countsketch/polars`, `kll/polars`, `dd/polars`, `topk/polars`). These are
//! real `--impl` rows, not accuracy baselines: they compute the exact answer
//! through a DataFrame engine, and the point of racing them is throughput.
//! Their exactness is incidental — the accuracy ground truth lives in
//! `accuracy/`.
//!
//! Mirrors the legacy `throughput/polars_{cardinality,freq,quantile}/`
//! binaries: buffer the stream into a `Vec<i64>`, then on
//! `finalize_for_query` build a `DataFrame` once and run the relevant Polars
//! expression. The runner bills that build to
//! `RunMetrics::finalize_wall_time_ns` and times the insert loop alone, so a
//! polars row's `throughput_items_per_sec` is the buffering `Vec::push`, not
//! the engine work. Its `build_throughput_items_per_sec` is push + DataFrame
//! + collect — the legacy number, and the one to race these rows on.
//!
//! The per-call estimate is a cached lookup, so under `--raw-csv --accuracy`
//! the per-call CSV rows report the post-finalize lookup cost (≈ ns), not the
//! Polars compute. That is what it should be: by the time the comparator
//! queries this row, the work is already done.

use std::collections::HashMap;

use aqpbm_core::accuracy::{CardinalityOps, FrequencyOps, QuantileOps, TopKOps};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::{CmsParams, CountSketchParams, DdParams, HllParams, KllParams, TopkParams};
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality {
    buf: Vec<i64>,
    estimate: f64,
}

impl InitSketch for PolarsCardinality {
    /// Polars computes the exact answer; it has no `(rows, cols)` to tune, so
    /// it ignores the `ParamSet` and builds unconditionally. Its record
    /// therefore carries whatever config the cell was given — typically the
    /// parameterless point, since there is nothing to set.
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
    fn finalize_for_query(&mut self) {
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
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsFrequencyCs {
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

impl Accumulator for PolarsQuantileKll {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn finalize_for_query(&mut self) {
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
    fn finalize_for_query(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsQuantileDd {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}


// ---------- statistic membership ----------
//
// The exact baselines answer the same statistics as the sketches they sit
// beside, which is what makes their ~0 error a check on the comparator.

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
//
// Each exact baseline is named `polars` inside whichever family its params
// type places it in — the family it belongs to falls out of `Params`.

impl BenchImpl for PolarsCardinality { type Params = HllParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsFrequencyCms { type Params = CmsParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsFrequencyCs { type Params = CountSketchParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsQuantileKll { type Params = KllParams; const IMPL: &'static str = "polars"; }
impl BenchImpl for PolarsQuantileDd { type Params = DdParams; const IMPL: &'static str = "polars"; }

/// `topk/polars` — the exact top-k baseline. Reuses the same group_by that
/// backs the frequency baseline, then sorts. Its score is the check on the
/// comparator itself: an exact answer must come back at precision = recall =
/// 1.0, so anything less means the ground-truth calculator, not the sketch, is wrong.
#[derive(Default)]
pub struct PolarsTopK(PolarsFrequencyCore);

impl InitSketch for PolarsTopK {
    /// The one polars baseline that does read its config. `k` is not a
    /// tuning knob it can shrug off like `rows`/`cols`: it is the prefix the
    /// comparator scores this row against, so a config whose `k` cannot be
    /// read has to fail here — naming the bad key — rather than let the row
    /// run and be silently scored at some other `k`. It also keeps the
    /// `topk` panel honest: every row in the family accepts and rejects the
    /// same configs, so the rows in a comparison were asked the same question.
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
    fn finalize_for_query(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn topk_config(params: serde_json::Value) -> ParamSet {
        ParamSet {
            family: "topk".to_string(),
            params,
        }
    }

    /// Exactness is no excuse for accepting a config the rest of the family
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
