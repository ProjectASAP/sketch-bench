//! Top-k family ground truth. Exact top-k from a HashMap
//! counter; reports precision@k and recall@k.

use crate::accumulator::Accumulator;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use super::statistic::TopKOps;
use super::{Comparison, GroundTruth};

pub struct TopkGT {
    pub k: usize,
}

impl<S, K> GroundTruth<S> for TopkGT
where
    K: Eq + Hash + Ord + Clone,
    S: Accumulator<Item = K> + TopKOps<Key = K>,
{
    fn compare(&self, sketch: &S, items: &[K]) -> Comparison {
        let mut exact: HashMap<K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it.clone()).or_insert(0) += 1;
        }
        let mut exact_vec: Vec<(K, u64)> = exact.into_iter().collect();
        // Count descending, ties on the key — a total order, and the same one
        // every top-k impl here ranks by. Sorting on the count alone leaves
        // the keys tied at the k-th place in `HashMap` iteration order, so
        // the "truth" set moved between processes and even between two calls,
        // and an exact source scored below 1.0 against itself.
        exact_vec.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        exact_vec.truncate(self.k);

        let q_start = Instant::now();
        let est: Vec<(K, u64)> = sketch.estimate_topk(self.k);
        let q_ns = q_start.elapsed().as_nanos() as u64;
        let est_set: std::collections::HashSet<K> = est.iter().map(|(k, _)| k.clone()).collect();
        let truth_set: std::collections::HashSet<K> =
            exact_vec.iter().map(|(k, _)| k.clone()).collect();

        let tp = est_set.intersection(&truth_set).count() as f64;
        let precision = if est.is_empty() {
            0.0
        } else {
            tp / est.len() as f64
        };
        let recall = if truth_set.is_empty() {
            0.0
        } else {
            tp / truth_set.len() as f64
        };

        Comparison {
            metrics: [
                ("k".to_string(), (self.k) as f64),
                ("precision_at_k".to_string(), (precision) as f64),
                ("recall_at_k".to_string(), (recall) as f64),
                ("true_top_k_count".to_string(), (truth_set.len()) as f64),
                ("est_top_k_count".to_string(), (est.len()) as f64),
            ]
            .into_iter()
            .collect::<BTreeMap<String, f64>>(),
            queries: 1,
            query_wall_ns: q_ns,
            query_calls: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    /// An exact top-k by hand: a `HashMap` counter, and a heap to pull the
    /// largest `k` off it.
    ///
    /// Exact by construction, so it must score 1.0 against [`TopkGT`] —
    /// anything less is the comparator being wrong, which is the whole point
    /// of the two tests below. Written here rather than borrowed from a real
    /// implementation so the comparator's own check owes nothing to a sketch
    /// library, or to whatever that library's notion of "top k" happens to be.
    #[derive(Default)]
    struct ExactTopK {
        counts: HashMap<i64, u64>,
    }

    impl Accumulator for ExactTopK {
        type Item = i64;
        fn update(&mut self, v: &i64) {
            *self.counts.entry(*v).or_insert(0) += 1;
        }
    }

    /// Ordered so the heap's max is "highest count, then *smallest* key" —
    /// count ascending, key descending. That is the same total order `TopkGT`
    /// ranks by; a source that broke ties the other way would score below 1.0
    /// and the tests could not tell that apart from a real defect.
    #[derive(PartialEq, Eq)]
    struct Ranked {
        count: u64,
        key: i64,
    }

    impl Ord for Ranked {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.count
                .cmp(&other.count)
                .then_with(|| other.key.cmp(&self.key))
        }
    }

    impl PartialOrd for Ranked {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl TopKOps for ExactTopK {
        type Key = i64;
        fn estimate_topk(&self, k: usize) -> Vec<(i64, u64)> {
            let mut heap: BinaryHeap<Ranked> = self
                .counts
                .iter()
                .map(|(&key, &count)| Ranked { count, key })
                .collect();
            std::iter::from_fn(|| heap.pop())
                .take(k)
                .map(|r| (r.key, r.count))
                .collect()
        }
    }

    /// Key 1 is heavy; keys 2..=6 all tie at 5. Scored at k = 3, the tie
    /// straddles the boundary — the one place a top-k ranking's tie-break is
    /// observable at all.
    fn tied_at_the_boundary() -> Vec<i64> {
        let mut items: Vec<i64> = std::iter::repeat(1).take(10).collect();
        for key in 2..=6 {
            items.extend(std::iter::repeat(key).take(5));
        }
        items
    }

    fn exact_source(items: &[i64]) -> ExactTopK {
        let mut s = ExactTopK::default();
        for it in items {
            s.update(it);
        }
        s
    }

    /// The exact baseline is this comparator's own check: anything below 1.0
    /// means the comparator is wrong, not the sketch. Repeated because the
    /// failure it guards was a per-`HashMap`-seed coin flip.
    #[test]
    fn an_exact_source_scores_precision_and_recall_of_one() {
        let items = tied_at_the_boundary();
        let gt = TopkGT { k: 3 };
        for _ in 0..32 {
            let cmp = gt.compare(&exact_source(&items), &items);
            assert_eq!(cmp.metrics["precision_at_k"], 1.0, "{:?}", cmp.metrics);
            assert_eq!(cmp.metrics["recall_at_k"], 1.0, "{:?}", cmp.metrics);
        }
    }

    /// Scoring is a pure function of `(sketch, items, k)`. It was not: the
    /// truth set was rebuilt per call from a fresh `HashMap`, so two calls on
    /// one sketch disagreed.
    #[test]
    fn scoring_the_same_sketch_twice_gives_the_same_answer() {
        let items = tied_at_the_boundary();
        let sketch = exact_source(&items);
        let gt = TopkGT { k: 3 };
        let first = gt.compare(&sketch, &items);
        for _ in 0..32 {
            assert_eq!(gt.compare(&sketch, &items).metrics, first.metrics);
        }
    }
}
