//! Top-k wrappers: a counter array plus a size-`k` candidate tracker, which a
//! bare Count-Min cannot answer — it stores counters, not keys. The tracker is
//! maintained on **every** update, so it sits in the insert hot path: that is
//! why this is its own row rather than a method on the CMS rows. Heavy-hitters
//! needs nothing on the insert side and belongs to the frequency rows.

use std::collections::HashMap;

use crate::params::TopkParams;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::accuracy::{FrequencyOps, TopKOps};
use aqpbm_core::config::{ParamSet, SketchParams};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;

/// A frequency sketch plus the size-`k` tracker that turns it into a top-k
/// sketch. Generic over the counter array so CMS and CountSketch share one
/// tracker — they differ only in what `estimate_frequency` costs.
pub struct TopKHeap<S> {
    inner: S,
    k: usize,
    /// The current candidate set, `key -> latest estimate`. A map, not a
    /// `BinaryHeap`, because the dominant operation is "is this key already a
    /// candidate" — O(k) on a heap, O(1) here.
    top: HashMap<i64, u64>,
    /// A cached **lower bound** on the smallest estimate in `top`, so a losing
    /// tail key costs one lookup and a compare. Only a bound — stale-low can
    /// skip the fast path, never admit wrongly.
    min_est: u64,
}

impl<S> TopKHeap<S>
where
    S: Accumulator<Item = i64> + FrequencyOps<Key = i64>,
{
    /// Offer `key`'s current estimate to the candidate set, maintaining
    /// `min_est <= min(top.values())` — every branch either restores it exactly
    /// or lowers it, so the fast reject stays conservative.
    #[inline(always)]
    fn offer(&mut self, key: i64) {
        let est = self.inner.estimate_frequency(&key);

        // Already a candidate: refresh its estimate. A rise leaves the bound
        // stale-low (a missed fast path); a fall would break it outright — a
        // CountSketch median can drop — so that direction is tracked, at O(1).
        if let Some(slot) = self.top.get_mut(&key) {
            *slot = est;
            self.min_est = self.min_est.min(est);
            return;
        }
        // Room left: take it unconditionally.
        if self.top.len() < self.k {
            self.top.insert(key, est);
            self.min_est = self.top.values().copied().min().unwrap_or(0);
            return;
        }
        // Losing to the bound means losing to the weakest candidate itself,
        // so this rejects without the scan. The overwhelmingly common case on
        // a skewed stream, and the only reason the bound is cached at all.
        if est <= self.min_est {
            return;
        }
        // Past the bound, promotion is decided against the weakest candidate's
        // *actual* estimate — on `min_est` alone a key beating a stale-low bound
        // could displace a far heavier one. Ties break on the key.
        let (weakest, weakest_est) = self
            .top
            .iter()
            .map(|(&k, &v)| (k, v))
            .min_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
            .expect("`top` is full, so it is non-empty");
        if est <= weakest_est {
            // The scan just proved the bound was stale; tightening it here is
            // what stops the next tail key paying for the same scan.
            self.min_est = weakest_est;
            return;
        }
        self.top.remove(&weakest);
        self.top.insert(key, est);
        self.min_est = self.top.values().copied().min().unwrap_or(0);
    }
}

impl<S> Accumulator for TopKHeap<S>
where
    S: Accumulator<Item = i64> + FrequencyOps<Key = i64>,
{
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
        self.offer(*v);
    }
}

impl<S> MemoryFootprint for TopKHeap<S>
where
    S: Accumulator<Item = i64> + FrequencyOps<Key = i64> + MemoryFootprint,
{
    fn memory_bytes(&self) -> usize {
        // The tracker is part of what this row costs, so it is billed here —
        // that is the trade the row exists to expose.
        self.inner.memory_bytes()
            + self.top.capacity() * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>())
    }
}

impl<S> TopKOps for TopKHeap<S> {
    type Key = i64;

    fn estimate_topk(&self, k: usize) -> Vec<(i64, u64)> {
        let mut out: Vec<(i64, u64)> = self.top.iter().map(|(&k, &c)| (k, c)).collect();
        // Descending by count; ties broken on the key so the ranking is
        // reproducible across runs, as the comparator's exact side also does.
        out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
    }
}

impl<S> InitSketch for TopKHeap<S>
where
    S: Accumulator<Item = i64> + FrequencyOps<Key = i64> + InitSketch + BenchImpl,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: TopkParams = config.parse()?;
        if p.k == 0 {
            return Err(BuildError("topk needs k >= 1".into()));
        }
        // The inner sketch is built from its **own** algorithm's params, so its
        // `init` validates `rows`/`cols` exactly as it does for a plain CMS
        // row — a fixed-shape inner still rejects a shape it cannot serve.
        let inner_cfg = ParamSet {
            algorithm: <S::Params as SketchParams>::ALGORITHM.to_string(),
            params: serde_json::json!({ "rows": p.rows, "cols": p.cols }),
        };
        Ok(Self {
            inner: S::init(&inner_cfg)?,
            k: p.k,
            top: HashMap::with_capacity(p.k),
            min_est: 0,
        })
    }
}

// ---------- catalog identity ----------
// One tracker, two rows: the `IMPL` name distinguishes which counter array is
// underneath.

impl BenchImpl for TopKHeap<super::cms::CmsOxide> {
    type Params = TopkParams;
    const IMPL: &'static str = "cms-heap";
}

impl BenchImpl for TopKHeap<super::countsketch::CsOxide> {
    type Params = TopkParams;
    const IMPL: &'static str = "cs-heap";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exact counter underneath, so anything these tests catch is tracker
    /// logic and not sketch error.
    #[derive(Default)]
    struct ExactCounter(HashMap<i64, u64>);

    impl Accumulator for ExactCounter {
        type Item = i64;
        fn update(&mut self, v: &i64) {
            *self.0.entry(*v).or_insert(0) += 1;
        }
    }

    impl MemoryFootprint for ExactCounter {
        fn memory_bytes(&self) -> usize {
            0
        }
    }

    impl FrequencyOps for ExactCounter {
        type Key = i64;
        fn estimate_frequency(&self, key: &i64) -> u64 {
            self.0.get(key).copied().unwrap_or(0)
        }
    }

    fn track(k: usize, stream: &[i64]) -> Vec<(i64, u64)> {
        let mut s = TopKHeap {
            inner: ExactCounter::default(),
            k,
            top: HashMap::with_capacity(k),
            min_est: 0,
        };
        for v in stream {
            s.update(v);
        }
        s.estimate_topk(k)
    }

    /// The bound is a lower bound, so "beats the bound" is not "beats the
    /// weakest candidate": key 20 at 2 clears a bound of 1 while key 10 sits at
    /// 50. Promoting on the bound alone evicts the heaviest key in the stream.
    #[test]
    fn a_light_key_does_not_displace_a_heavy_one() {
        let mut stream: Vec<i64> = std::iter::repeat(10).take(50).collect();
        stream.extend([20, 20]);
        assert_eq!(track(1, &stream), vec![(10, 50)]);
    }

    /// The same failure one rank down: 30 (count 2) must not push out 20
    /// (count 4) just because the bound is still sitting at the estimate 20
    /// had when it was admitted.
    #[test]
    fn eviction_picks_the_weakest_candidate_not_the_stalest_bound() {
        let stream = [10, 20, 10, 10, 10, 10, 20, 20, 20, 30, 30];
        assert_eq!(track(2, &stream), vec![(10, 5), (20, 4)]);
    }

    /// Three candidates tied at the minimum, one promotion: which one leaves
    /// must not depend on `HashMap::iter` order, or the same workload yields
    /// different candidate sets per process. Repeated for the per-map seed.
    #[test]
    fn eviction_among_tied_candidates_is_reproducible() {
        let stream = [1, 2, 3, 4, 4];
        let first = track(3, &stream);
        for _ in 0..32 {
            assert_eq!(track(3, &stream), first);
        }
        // The tie breaks on the key, so key 1 is the one that leaves.
        assert_eq!(first, vec![(4, 2), (2, 1), (3, 1)]);
    }

    /// The same sequences through the real `topk/cms-heap` row: a counter
    /// array wide enough that the three keys cannot collide, so the tracker
    /// is still the only thing under test — but reached through `init`.
    #[test]
    fn the_cms_backed_row_ranks_the_stream_correctly() {
        let cfg = ParamSet {
            algorithm: "topk".to_string(),
            params: serde_json::json!({ "rows": 5, "cols": 4096, "k": 2 }),
        };
        let mut s = TopKHeap::<super::super::cms::CmsOxide>::init(&cfg).unwrap();
        for v in [10, 20, 10, 10, 10, 10, 20, 20, 20, 30, 30] {
            s.update(&v);
        }
        assert_eq!(s.estimate_topk(2), vec![(10, 5), (20, 4)]);
    }
}
