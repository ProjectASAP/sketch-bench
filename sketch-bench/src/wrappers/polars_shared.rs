//! The pieces the `polars` baselines share: a frequency core, a quantile
//! core, and the subset fan-out every grouped baseline needs. Hoisted here
//! when the baselines moved next to the algorithms they score.

use ::polars::prelude::*;
use std::collections::HashMap;

/// the resulting key→count table for O(1) per-key queries.
/// Shared by `cms/polars` and `countsketch/polars`.
#[derive(Default)]
pub struct PolarsFrequencyCore {
    buf: Vec<i64>,
    counts: HashMap<i64, u64>,
}

impl PolarsFrequencyCore {
    #[inline(always)]
    pub fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }
    pub fn finalize(&mut self) {
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
        for (k, c) in keys.into_iter().zip(counts) {
            if let (Some(k), Some(c)) = (k, c) {
                self.counts.insert(k, c);
            }
        }
    }
    pub fn query(&self, q: i64) -> u64 {
        self.counts.get(&q).copied().unwrap_or(0)
    }
    pub fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<i64>()
            + self.counts.capacity() * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>())
    }
}

/// A value the `polars` baselines can build a column out of, at the width it
/// was ingested at. Declared here, beside the only rows that build columns,
/// rather than on the ordering trait the sketch rows share: `dd` has no
/// `polars` row, and bolting this onto `QuantileValue` would have made every
/// DDSketch wrapper carry a column-building method it never calls.
pub trait PolarsColumnItem: Copy {
    fn polars_column(values: &[Self]) -> Column;
}

impl PolarsColumnItem for i64 {
    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }
}

impl PolarsColumnItem for u64 {
    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }
}

impl PolarsColumnItem for f64 {
    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }
}

/// Polars-backed quantile baseline. The heavy work — one sort plus a 101-point
/// quantile grid — lives in `prepare`, which the runner times separately.
/// Insert is pure `Vec::push`; per-call `query()` is an array lookup.
///
/// Generic over the ingested width: this row is what the sketch rows are scored
/// against, so a width they build at and this one did not would leave their
/// error unattributable. The `Dtype` match in each row is what keeps the two
/// sets aligned.
pub struct PolarsQuantileCore<T: PolarsColumnItem = i64> {
    buf: Vec<T>,
    quantiles: [f64; 101],
}

impl<T: PolarsColumnItem> Default for PolarsQuantileCore<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            quantiles: [0.0; 101],
        }
    }
}

impl<T: PolarsColumnItem> PolarsQuantileCore<T> {
    #[inline(always)]
    pub fn update(&mut self, v: &T) {
        self.buf.push(*v);
    }
    pub fn finalize(&mut self) {
        let series = T::polars_column(&self.buf);
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
    pub fn query(&self, p: f64) -> f64 {
        let i = (p * 100.0).round().clamp(0.0, 100.0) as usize;
        self.quantiles[i]
    }
    pub fn memory_bytes(&self) -> usize {
        self.buf.capacity() * std::mem::size_of::<T>()
    }
}

/// The `;`-joined key of one label subset, in column order. This is the format
/// `Hydra::update` builds internally, reproduced so the baseline and the sketch
/// answer to the same key.
pub fn subset_key(parts: &[&str], mask: usize) -> String {
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
pub fn fan_out<V: Copy>(key: &str, value: V, keys: &mut Vec<String>, values: &mut Vec<V>) {
    let parts: Vec<&str> = key.split(';').filter(|s| !s.is_empty()).collect();
    for mask in 1..(1usize << parts.len()) {
        keys.push(subset_key(&parts, mask));
        values.push(value);
    }
}
