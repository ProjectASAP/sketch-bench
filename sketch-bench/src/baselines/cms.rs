//! Exact frequency baseline — `HashMap<i64, u64>`, freq =
//! `map.get(k)`. Implements `Sketch<Item=i64, Query=i64, Answer=u64>`.

use std::collections::HashMap;

use sketch_core::config::CmsParams;
use sketch_core::sketch::Sketch;

/// Matches the threshold used by `accuracy/cms/rust/src/baseline.rs`
/// for heavy-hitter sets — true count ≥ this is kept.
pub const HEAVY_HITTER_MIN_TRUE_COUNT: u64 = 100;

#[derive(Debug, Default, Clone)]
pub struct ExactCms {
    map: HashMap<i64, u64>,
}

impl ExactCms {
    /// Accepts a `CmsParams` for dispatch-macro uniformity; the
    /// value is ignored — an exact counter has no shape.
    pub fn new(_p: &CmsParams) -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Batch ingest, mirroring the old accuracy-harness load.
    pub fn ingest_all(values: &[i64]) -> Self {
        let mut this = Self {
            map: HashMap::with_capacity(values.len().min(1 << 20)),
        };
        for v in values {
            *this.map.entry(*v).or_insert(0) += 1;
        }
        this
    }

    pub fn distinct_items(&self) -> usize {
        self.map.len()
    }

    /// Keys whose true count is at least `min_count`. Preserves
    /// the shape used by `accuracy/cms/rust/src/baseline.rs`.
    pub fn heavy_hitters(&self, min_count: u64) -> Vec<(i64, u64)> {
        self.map
            .iter()
            .filter_map(|(&k, &c)| (c >= min_count).then_some((k, c)))
            .collect()
    }

    /// Access the underlying frequency map (immutable). Lets the
    /// accuracy harness iterate / compute derived stats without
    /// copying.
    pub fn frequencies(&self) -> &HashMap<i64, u64> {
        &self.map
    }
}

impl Sketch for ExactCms {
    type Item = i64;
    type Query = i64;
    type Answer = u64;

    fn update(&mut self, v: &i64) {
        *self.map.entry(*v).or_insert(0) += 1;
    }

    fn query(&self, q: i64) -> u64 {
        self.map.get(&q).copied().unwrap_or(0)
    }

    fn memory_bytes(&self) -> usize {
        self.map.capacity()
            * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_freq_lookup() {
        let vals = [1i64, 2, 2, 3, 3, 3, 4];
        let b = ExactCms::ingest_all(&vals);
        assert_eq!(b.query(1), 1);
        assert_eq!(b.query(2), 2);
        assert_eq!(b.query(3), 3);
        assert_eq!(b.query(4), 1);
        assert_eq!(b.query(999), 0);
        assert_eq!(b.distinct_items(), 4);
    }

    #[test]
    fn heavy_hitters_honours_threshold() {
        let vals: Vec<i64> = std::iter::repeat(7).take(150).chain([8, 8, 9]).collect();
        let b = ExactCms::ingest_all(&vals);
        let hh = b.heavy_hitters(100);
        assert_eq!(hh, vec![(7, 150)]);
    }
}
