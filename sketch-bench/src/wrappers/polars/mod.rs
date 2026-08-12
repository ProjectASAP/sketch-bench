//! Polars-backed exact implementations, one file per row. They compute the true
//! answer through a DataFrame engine, so their error is ~0 and a non-zero one is
//! a finding about the harness, not an approximation. Every one buffers the
//! stream in `update` and does its work in `prepare`, so race them on
//! `build_throughput_items_per_sec` — the insert column is the `Vec::push`.

use std::collections::HashMap;

use polars::prelude::*;

pub mod polars_cardinality;
pub mod polars_frequency_cms;
pub mod polars_frequency_cs;
pub mod polars_quantile_dd;
pub mod polars_quantile_kll;
pub mod polars_subpop_cardinality;
pub mod polars_subpop_frequency;
pub mod polars_subpop_quantile;
pub mod polars_topk;

pub use polars_cardinality::PolarsCardinality;
pub use polars_frequency_cms::PolarsFrequencyCms;
pub use polars_frequency_cs::PolarsFrequencyCs;
pub use polars_quantile_dd::PolarsQuantileDd;
pub use polars_quantile_kll::PolarsQuantileKll;
pub use polars_subpop_cardinality::PolarsSubpopCardinality;
pub use polars_subpop_frequency::PolarsSubpopFrequency;
pub use polars_subpop_quantile::PolarsSubpopQuantile;
pub use polars_topk::PolarsTopK;

/// Polars-backed frequency: `group_by(v).agg(len)`, then cache
/// the resulting key→count table for O(1) per-key queries.
/// Shared by `cms/polars` and `countsketch/polars`.
#[derive(Default)]
pub(super) struct PolarsFrequencyCore {
    buf: Vec<i64>,
    pub(super) counts: HashMap<i64, u64>,
}

impl PolarsFrequencyCore {
    #[inline(always)]
    pub(super) fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    pub(super) fn finalize(&mut self) {
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
    pub(super) fn query(&self, q: i64) -> u64 {
        self.counts.get(&q).copied().unwrap_or(0)
    }
    pub(super) fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
            + self.counts.capacity() * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>())
    }
}

/// Polars-backed quantile baseline. The heavy work — one sort plus a 101-point
/// quantile grid — lives in `prepare`, which the runner times separately.
/// Insert is pure `Vec::push`; per-call `query()` is an array lookup.
pub(super) struct PolarsQuantileCore {
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
    pub(super) fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    pub(super) fn finalize(&mut self) {
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
    pub(super) fn query(&self, p: f64) -> f64 {
        let i = (p * 100.0).round().clamp(0.0, 100.0) as usize;
        self.quantiles[i]
    }
    pub(super) fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

// ---------- the subset key space, shared by the three grouped rows ----------
//
// A grouped sketch writes each record into every non-empty subset of its labels,
// so a baseline grouping by one column would answer an easier question. These
// reproduce the key space exactly, which is why two label columns sharing a
// value collide here too — a non-zero error on a grouped row is that aliasing
// (#74) and not an approximation.

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
