//! Exact cardinality baseline — `HashSet<i64>`, cardinality =
//! `set.len()`. Provides ground truth for any sketch family that
//! answers a cardinality query.
//!
//! Current consumers: `hll` (see `baselines::Statistic::Cardinality`).

use std::collections::HashSet;

use sketch_core::config::HllParams;
use sketch_core::sketch::Sketch;

#[derive(Debug, Default, Clone)]
pub struct ExactCardinality {
    set: HashSet<i64>,
}

impl ExactCardinality {
    /// Accepts an `HllParams` so it slots into the same dispatch
    /// macros as the sketch impls. The value is ignored — an
    /// exact cardinality count has no tuning knobs.
    pub fn new(_p: &HllParams) -> Self {
        Self {
            set: HashSet::new(),
        }
    }

    /// Batch ingest — used by the `accuracy/` harness to mirror
    /// the old `BaselineData` load path.
    pub fn ingest_all(values: &[i64]) -> Self {
        let mut this = Self {
            set: HashSet::with_capacity(values.len().min(1 << 20)),
        };
        for v in values {
            this.set.insert(*v);
        }
        this
    }

    /// Exact distinct-item count.
    pub fn distinct_items(&self) -> usize {
        self.set.len()
    }
}

impl Sketch for ExactCardinality {
    type Item = i64;
    type Query = ();
    type Answer = f64;

    fn update(&mut self, v: &i64) {
        self.set.insert(*v);
    }

    fn query(&self, _: ()) -> f64 {
        self.set.len() as f64
    }

    fn memory_bytes(&self) -> usize {
        self.set.capacity() * (std::mem::size_of::<i64>() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_cardinality_matches_hashset_len() {
        let vals = [1i64, 2, 2, 3, 3, 3, 4];
        let b = ExactCardinality::ingest_all(&vals);
        assert_eq!(b.distinct_items(), 4);
        assert_eq!(b.query(()), 4.0);
    }

    #[test]
    fn update_and_batch_agree() {
        let vals: Vec<i64> = (0..1000).flat_map(|i| [i, i, i]).collect();
        let batch = ExactCardinality::ingest_all(&vals);
        let mut streamed = ExactCardinality::new(&HllParams { lg_k: 14 });
        for v in &vals {
            streamed.update(v);
        }
        assert_eq!(batch.distinct_items(), streamed.distinct_items());
    }
}
