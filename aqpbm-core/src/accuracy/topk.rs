//! Top-k algorithm ground truth. Exact top-k from a HashMap
//! counter; reports precision@k and recall@k.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;

use super::GroundTruth;

pub struct TopkGT {
    pub k: usize,
}

impl<K> GroundTruth<K> for TopkGT
where
    K: Eq + Hash + Ord + Clone,
{
    /// The true k heaviest keys, in the total order every top-k impl ranks by.
    type Truth = Vec<(K, u64)>;
    /// One question: the whole list. A probe carries nothing.
    type Probe = ();
    type Answer = Vec<(K, u64)>;

    fn truth(&self, items: &[K]) -> Vec<(K, u64)> {
        let mut exact: HashMap<K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it.clone()).or_insert(0) += 1;
        }
        let mut exact_vec: Vec<(K, u64)> = exact.into_iter().collect();
        // Count descending, ties on the key — a total order, the same one every
        // top-k impl ranks by. On count alone the k-th place falls to HashMap
        // order, and an exact source scores below 1.0 against itself.
        exact_vec.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        exact_vec.truncate(self.k);
        exact_vec
    }

    fn probes(&self, _truth: &Vec<(K, u64)>) -> Vec<()> {
        vec![()]
    }

    fn score(
        &self,
        truth: &Vec<(K, u64)>,
        _probes: &[()],
        answers: &[Vec<(K, u64)>],
    ) -> BTreeMap<String, f64> {
        let empty = Vec::new();
        let est = answers.first().unwrap_or(&empty);
        let est_set: std::collections::HashSet<&K> = est.iter().map(|(k, _)| k).collect();
        let truth_set: std::collections::HashSet<&K> = truth.iter().map(|(k, _)| k).collect();

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

        [
            ("k".to_string(), self.k as f64),
            ("precision_at_k".to_string(), precision),
            ("recall_at_k".to_string(), recall),
            ("true_top_k_count".to_string(), truth_set.len() as f64),
            ("est_top_k_count".to_string(), est.len() as f64),
        ]
        .into_iter()
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    /// An exact top-k by hand: a `HashMap` counter plus a heap. Exact by
    /// construction, so it must score 1.0 against [`TopkGT`]; anything less is
    /// the comparator being wrong. Hand-written so the check owes no library.
    #[derive(Default, Clone)]
    struct ExactTopK {
        counts: HashMap<i64, u64>,
    }

    impl ExactTopK {
        fn update(&mut self, v: &i64) {
            *self.counts.entry(*v).or_insert(0) += 1;
        }
    }

    /// Ordered so the heap's max is "highest count, then *smallest* key" — the
    /// same total order `TopkGT` ranks by. Breaking ties the other way scores
    /// below 1.0, which the tests could not distinguish from a real defect.
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

    impl ExactTopK {
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

    /// How the double is asked. `k` is the comparator's, so the closure reads
    /// it from the probe's own comparator rather than from the sketch.
    fn ask_exact(s: &mut ExactTopK, _: &()) -> Vec<(i64, u64)> {
        s.estimate_topk(3)
    }

    /// The exact baseline is this comparator's own check: anything below 1.0
    /// means the comparator is wrong, not the sketch. Repeated because the
    /// failure it guards was a per-`HashMap`-seed coin flip.
    #[test]
    fn an_exact_source_scores_precision_and_recall_of_one() {
        let items = tied_at_the_boundary();
        let gt = TopkGT { k: 3 };
        for _ in 0..32 {
            let cmp = crate::accuracy::run_probes(&gt, &ask_exact, &mut exact_source(&items), &items, false);
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
        let first = crate::accuracy::run_probes(&gt, &ask_exact, &mut sketch.clone(), &items, false);
        for _ in 0..32 {
            assert_eq!(
                crate::accuracy::run_probes(&gt, &ask_exact, &mut sketch.clone(), &items, false)
                    .metrics,
                first.metrics
            );
        }
    }
}
