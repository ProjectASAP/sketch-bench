//! Exact frequency baseline — `HashMap<i64, u64>`, freq =
//! `map.get(k)`. Provides ground truth for any sketch family that
//! answers a per-key frequency query or derives heavy hitters
//! from per-key counts.
//!
//! Current consumers: `cms`, `countsketch`, `elastic` (see
//! `baselines::Statistic::Frequency`).

use hashbrown::HashMap;

use crate::init::{BuildError, InitSketch};
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::{MergeUnsupported, Sketch};

/// Heavy-hitter threshold — matches the value the legacy
/// `accuracy/cms/rust/src/baseline.rs` used. Keys with true
/// count ≥ this are kept in `heavy_hitters`.
pub const HEAVY_HITTER_MIN_TRUE_COUNT: u64 = 100;

#[derive(Debug, Default, Clone)]
pub struct ExactFrequency {
    map: HashMap<i64, u64>,
}

impl ExactFrequency {
    /// Accepts the family's params by reference, ignored, for
    /// dispatch-macro uniformity; the
    /// value is ignored — an exact counter has no shape.
    pub fn new<P>(_p: &P) -> Self {
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

impl Sketch for ExactFrequency {
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
        // hashbrown reports the SwissTable buffer exactly: bucket
        // array + control bytes + group padding. Excludes the
        // struct's stack footprint, which is fine — `memory_bytes`
        // is documented as the heap-resident core (see Sketch).
        self.map.allocation_size()
    }

    /// Counter-wise addition, exactly what a linear frequency sketch does.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        for (k, v) in other.map.iter() {
            *self.map.entry(*k).or_insert(0) += *v;
        }
        Ok(())
    }
}

impl InitSketch for ExactFrequency {
    fn init(_config: &ParamSet) -> Result<Self, BuildError> {
        Err(BuildError(crate::init::BASELINE_NO_PARAM_SPACE.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_freq_lookup() {
        let vals = [1i64, 2, 2, 3, 3, 3, 4];
        let b = ExactFrequency::ingest_all(&vals);
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
        let b = ExactFrequency::ingest_all(&vals);
        let hh = b.heavy_hitters(100);
        assert_eq!(hh, vec![(7, 150)]);
    }
}
