//! `hydra-cms/polars` — exact subpopulation frequency over every label subset.

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use polars::prelude::*;
use aqpbm_core::workload::Labeled;
use std::collections::HashMap;

use super::fan_out;
use aqpbm_core::accuracy::SubpopFrequencyOps;
use crate::params::HydraCmsParams;

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

impl Accumulator for PolarsSubpopFrequency {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, value: i64) -> Labeled<i64> {
        Labeled { key: key.to_string(), value }
    }

    fn built() -> PolarsSubpopFrequency {
        PolarsSubpopFrequency::init(&ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: 64,
            cell_rows: 3,
            cell_cols: 256,
        }))
        .expect("canonical dimensions build")
    }

    /// Folding two shards must give the answer the whole stream gives. Exact on
    /// both sides, so this is an equality and not a tolerance.
    #[test]
    fn merging_shards_matches_the_whole_stream() {
        let left_items = [record("a;x", 10), record("a;y", 10)];
        let right_items = [record("a;x", 10), record("b;x", 20)];

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

        for (labels, value) in [
            (vec!["a"], 10),
            (vec!["a", "x"], 10),
            (vec!["b"], 20),
            (vec!["zzz"], 10),
        ] {
            assert_eq!(
                left.estimate_subpop_frequency(&labels, &value),
                whole.estimate_subpop_frequency(&labels, &value),
                "merged and whole-stream disagree at {labels:?}/{value}"
            );
        }
        // The depth-1 group `a` covers three records across both shards.
        assert_eq!(left.estimate_subpop_frequency(&["a"], &10), 3.0);
    }
}
