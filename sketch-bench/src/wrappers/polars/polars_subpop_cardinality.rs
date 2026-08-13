//! `hydra-hll/polars` — exact subpopulation cardinality over every label subset.

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
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

    /// The exact control for the merge square. The runner fills shards with
    /// `update` only and calls `prepare` after the fold, so concatenating the
    /// buffers is the whole of it — the DataFrame work then runs over the whole
    /// stream. Lossless by construction, which is what makes it the control:
    /// any gap on the sketch beside it is the sketch's.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.buf.extend_from_slice(&other.buf);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, value: i64) -> Labeled<i64> {
        Labeled { key: key.to_string(), value }
    }

    fn built() -> PolarsSubpopCardinality {
        PolarsSubpopCardinality::init(&ParamSet::of(&HydraHllParams { rows: 3, cols: 64 }))
            .expect("canonical dimensions build")
    }

    /// The value shared across the two shards must be counted once, which is
    /// the property a concatenating merge has to preserve for a distinct count.
    #[test]
    fn merging_shards_does_not_double_count() {
        let left_items = [record("a;x", 10), record("a;x", 20)];
        let right_items = [record("a;x", 20), record("a;x", 30)];

        let (mut left, mut right) = (built(), built());
        for r in &left_items {
            left.update(r);
        }
        for r in &right_items {
            right.update(r);
        }
        left.merge(&right).expect("the exact control merges");
        left.prepare();

        let mut whole = built();
        for r in left_items.iter().chain(right_items.iter()) {
            whole.update(r);
        }
        whole.prepare();

        // 10, 20, 30 — three distinct, though 20 occurs in both shards.
        assert_eq!(left.estimate_subpop_cardinality(&["a"]), 3.0);
        assert_eq!(
            left.estimate_subpop_cardinality(&["a"]),
            whole.estimate_subpop_cardinality(&["a"])
        );
    }
}
