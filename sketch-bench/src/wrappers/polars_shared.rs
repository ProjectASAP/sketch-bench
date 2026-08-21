//! The pieces the `polars` baselines share: a frequency core, a quantile
//! core, and the subset fan-out every grouped baseline needs. Hoisted here
//! when the baselines moved next to the algorithms they score.

use crate::wrappers::hll::CardinalityValue;
use ::polars::prelude::*;
use std::collections::HashMap;

/// the resulting key→count table for O(1) per-key queries.
/// Shared by `cms/polars` and `countsketch/polars`.
///
/// Generic over the ingested width for the same reason the quantile core below
/// is: this row is the exact baseline its sketch siblings are scored against,
/// so a width they build at and this one did not would leave their error with
/// nothing to subtract.
pub struct PolarsFrequencyCore<T: PolarsFrequencyItem = i64> {
    buf: Vec<T>,
    counts: HashMap<T::CountKey, u64>,
}

impl<T: PolarsFrequencyItem> Default for PolarsFrequencyCore<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            counts: HashMap::new(),
        }
    }
}

impl<T: PolarsFrequencyItem> PolarsFrequencyCore<T> {
    #[inline(always)]
    pub fn update(&mut self, v: &T) {
        self.buf.push(v.clone());
    }
    pub fn finalize(&mut self) {
        let series = T::polars_column(&self.buf);
        let df = DataFrame::new(vec![series]).expect("DataFrame::new");
        let result = df
            .lazy()
            .group_by([col("v")])
            .agg([len().alias("count")])
            .collect()
            .expect("polars group_by collect");
        let keys = T::count_keys(result.column("v").expect("v column"));
        let counts = result
            .column("count")
            .expect("count column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");
        self.counts.reserve(keys.len());
        for (k, c) in keys.into_iter().zip(counts) {
            if let Some(c) = c {
                self.counts.insert(k, c);
            }
        }
    }
    pub fn query(&self, q: &T) -> u64 {
        self.counts.get(&q.count_key()).copied().unwrap_or(0)
    }
    pub fn memory_bytes(&self) -> usize {
        let keys: usize = self.buf.iter().map(T::heap_bytes).sum();
        self.buf.capacity() * std::mem::size_of::<T>()
            + keys
            + self.counts.capacity()
                * (std::mem::size_of::<T::CountKey>() + std::mem::size_of::<u64>())
    }
}

/// What the exact frequency baseline needs on top of building a column: an
/// owned key it can count on, and how to read one back out of what `group_by`
/// answered. Its own trait rather than more methods on [`PolarsColumnItem`],
/// so the quantile baseline does not carry a counting key it never uses.
pub trait PolarsFrequencyItem: PolarsColumnItem + 'static {
    /// `Self` for the widths that can key a map; the bit pattern for `f64`,
    /// matching how the frequency wrappers and the exact scorer both key it.
    type CountKey: Eq + std::hash::Hash + 'static;

    fn count_key(&self) -> Self::CountKey;

    /// Peel the grouped key column back at this width.
    fn count_keys(column: &Column) -> Vec<Self::CountKey>;

    /// Bytes this value owns away from the `Vec` that holds it. Zero for the
    /// numeric widths; the allocation for `String`, which `size_of` misses.
    fn heap_bytes(&self) -> usize;
}

impl PolarsFrequencyItem for i64 {
    type CountKey = i64;
    #[inline(always)]
    fn count_key(&self) -> i64 {
        *self
    }
    fn count_keys(column: &Column) -> Vec<i64> {
        column
            .i64()
            .expect("i64 keys")
            .into_iter()
            .flatten()
            .collect()
    }
    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl PolarsFrequencyItem for u64 {
    type CountKey = u64;
    #[inline(always)]
    fn count_key(&self) -> u64 {
        *self
    }
    fn count_keys(column: &Column) -> Vec<u64> {
        column
            .u64()
            .expect("u64 keys")
            .into_iter()
            .flatten()
            .collect()
    }
    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl PolarsFrequencyItem for f64 {
    type CountKey = u64;
    #[inline(always)]
    fn count_key(&self) -> u64 {
        self.to_bits()
    }
    fn count_keys(column: &Column) -> Vec<u64> {
        column
            .f64()
            .expect("f64 keys")
            .into_iter()
            .flatten()
            .map(f64::to_bits)
            .collect()
    }
    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl PolarsFrequencyItem for String {
    type CountKey = String;
    #[inline(always)]
    fn count_key(&self) -> String {
        self.clone()
    }
    fn count_keys(column: &Column) -> Vec<String> {
        column
            .str()
            .expect("string keys")
            .into_iter()
            .flatten()
            .map(str::to_string)
            .collect()
    }
    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
}

/// A value the `polars` baselines can build a column out of, at the width it
/// was ingested at. Declared here, beside the only rows that build columns,
/// rather than on the ordering trait the sketch rows share: `dd` has no
/// `polars` row, and bolting this onto `QuantileValue` would have made every
/// DDSketch wrapper carry a column-building method it never calls.
pub trait PolarsColumnItem: Clone {
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

impl PolarsColumnItem for String {
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
        self.buf.push(v.clone());
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
pub fn fan_out<V: Clone>(key: &str, value: &V, keys: &mut Vec<String>, values: &mut Vec<V>) {
    let parts: Vec<&str> = key.split(';').filter(|s| !s.is_empty()).collect();
    for mask in 1..(1usize << parts.len()) {
        keys.push(subset_key(&parts, mask));
        values.push(value.clone());
    }
}

pub struct PolarsSubpopFrequencyCore<T: PolarsFrequencyItem = i64> {
    buf: Vec<(String, T)>,
    counts: HashMap<(String, T::CountKey), u64>,
}

impl<T: PolarsFrequencyItem> Default for PolarsSubpopFrequencyCore<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            counts: HashMap::new(),
        }
    }
}

impl<T: PolarsFrequencyItem> PolarsSubpopFrequencyCore<T> {
    #[inline(always)]
    pub fn update(&mut self, record: &(String, T)) {
        self.buf.push(record.clone());
    }

    pub fn finalize(&mut self) {
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &self.buf {
            fan_out(&r.0, &r.1, &mut keys, &mut values);
        }
        if keys.is_empty() {
            return;
        }
        let df = DataFrame::new(vec![
            Column::new("g".into(), &keys),
            T::polars_column(&values),
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
        let vals = T::count_keys(result.column("v").expect("v column"));
        let counts = result
            .column("count")
            .expect("count column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");

        self.counts.reserve(groups.len());
        for ((g, v), c) in groups.into_iter().zip(vals).zip(counts) {
            if let (Some(g), Some(c)) = (g, c) {
                self.counts.insert((g.to_string(), v), c);
            }
        }
    }

    pub fn query(&self, labels: &[&str], value: &T) -> f64 {
        self.counts
            .get(&(labels.join(";"), value.count_key()))
            .copied()
            .unwrap_or(0) as f64
    }

    pub fn memory_bytes(&self) -> usize {
        let keys: usize = self
            .buf
            .iter()
            .map(|(g, v)| g.capacity() + v.heap_bytes())
            .sum();
        self.buf.capacity() * std::mem::size_of::<(String, T)>()
            + keys
            + self.counts.capacity()
                * (std::mem::size_of::<(String, T::CountKey)>() + std::mem::size_of::<u64>())
    }
}

pub struct PolarsSubpopCardinalityCore<T: CardinalityValue = i64> {
    buf: Vec<(String, T)>,
    distinct: HashMap<String, u64>,
}

impl<T: CardinalityValue> Default for PolarsSubpopCardinalityCore<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            distinct: HashMap::new(),
        }
    }
}

impl<T: CardinalityValue> PolarsSubpopCardinalityCore<T> {
    #[inline(always)]
    pub fn update(&mut self, record: &(String, T)) {
        self.buf.push(record.clone());
    }

    /// The exact answer, over the stream as buffered. Its own step because the
    /// runner times it as `prepare`: this row's cost is the pass, not the ask.
    pub fn finalize(&mut self) {
        let Some(result) = grouped_values(&self.buf, |lazy| {
            lazy.group_by([col("g")])
                .agg([col("v").n_unique().alias("c")])
        }) else {
            return;
        };
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

    pub fn query(&self, labels: &[&str]) -> f64 {
        self.distinct.get(&labels.join(";")).copied().unwrap_or(0) as f64
    }

    pub fn memory_bytes(&self) -> usize {
        let held: usize = self
            .buf
            .iter()
            .map(|(g, v)| g.capacity() + v.heap_bytes())
            .sum();
        self.buf.capacity() * std::mem::size_of::<(String, T)>()
            + held
            + self.distinct.capacity() * (std::mem::size_of::<String>() + 8)
    }
}

pub struct PolarsSubpopVectorCore<T: CardinalityValue = i64> {
    buf: Vec<(String, T)>,
    exact: HashMap<String, f64>,
    fold: fn(&[u64]) -> f64,
}

impl<T: CardinalityValue> PolarsSubpopVectorCore<T> {
    pub fn folding(fold: fn(&[u64]) -> f64) -> Self {
        Self {
            buf: Vec::new(),
            exact: HashMap::new(),
            fold,
        }
    }

    #[inline(always)]
    pub fn update(&mut self, record: &(String, T)) {
        self.buf.push(record.clone());
    }

    /// The exact answer, over the stream as buffered. Its own step because the
    /// runner times it as `prepare`: this row's cost is the pass, not the ask.
    pub fn finalize(&mut self) {
        let Some(result) = grouped_values(&self.buf, |lazy| {
            lazy.group_by([col("g"), col("v")])
                .agg([len().alias("count")])
        }) else {
            return;
        };
        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let counts = result
            .column("count")
            .expect("count column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");

        let mut per_group: HashMap<String, Vec<u64>> = HashMap::new();
        for (g, c) in groups.into_iter().zip(counts) {
            if let (Some(g), Some(c)) = (g, c) {
                per_group.entry(g.to_string()).or_default().push(c);
            }
        }
        self.exact = per_group
            .into_iter()
            .map(|(g, counts)| {
                let folded = (self.fold)(&counts);
                (g, folded)
            })
            .collect();
    }

    pub fn query(&self, labels: &[&str]) -> f64 {
        self.exact.get(&labels.join(";")).copied().unwrap_or(0.0)
    }

    pub fn memory_bytes(&self) -> usize {
        let held: usize = self
            .buf
            .iter()
            .map(|(g, v)| g.capacity() + v.heap_bytes())
            .sum();
        self.buf.capacity() * std::mem::size_of::<(String, T)>()
            + held
            + self.exact.capacity() * (std::mem::size_of::<String>() + std::mem::size_of::<f64>())
    }
}

fn grouped_values<T: CardinalityValue>(
    buf: &[(String, T)],
    aggregate: impl Fn(LazyFrame) -> LazyFrame,
) -> Option<DataFrame> {
    let (mut keys, mut values) = (Vec::new(), Vec::new());
    for r in buf {
        fan_out(&r.0, &r.1, &mut keys, &mut values);
    }
    if keys.is_empty() {
        return None;
    }
    let df = DataFrame::new(vec![
        Column::new("g".into(), &keys),
        T::polars_column(&values),
    ])
    .expect("DataFrame::new");
    Some(
        aggregate(df.lazy())
            .collect()
            .expect("polars group_by collect"),
    )
}
