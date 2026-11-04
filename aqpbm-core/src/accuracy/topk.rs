//! Top-k algorithm ground truth. Exact top-k from a HashMap
//! counter; reports precision@k and recall@k.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{CountedValue, GroundTruth, StreamStats};

/// How many times to put the one question. A heap dump of `k` items takes
/// about a microsecond, so with one call per pass the CPU time read from
/// `getrusage` around it is mostly the cost of the reading itself. As for
/// `CardinalityGT`, the count is this statistic's knowledge, not the runner's.
const QUERY_TIMING_REPEATS: usize = 1024;

pub struct TopkGT<K> {
    pub k: usize,
    pub column: usize,
    key: PhantomData<K>,
}

impl<K> TopkGT<K> {
    pub fn over_column(k: usize, column: usize) -> Self {
        Self {
            k,
            column,
            key: PhantomData,
        }
    }
}

pub struct TopkTruth<K: CountedValue> {
    top: Vec<(K, u64)>,
    exact: HashMap<K::CountKey, (K, u64)>,
    stream: StreamStats,
}

impl<K> GroundTruth for TopkGT<K>
where
    K: CountedValue,
{
    /// The true k heaviest keys, in the total order every top-k impl ranks by.
    type Truth = TopkTruth<K>;
    /// One question, the whole list, asked repeatedly. A probe carries nothing.
    type Probe = ();
    type Answer = Vec<(K, u64)>;

    fn truth(&self, table: &GeneratedTable) -> Result<TopkTruth<K>, DataGenError> {
        let items = K::column_slice(table.column(self.column)?)?;
        // Tallied by counting key so the float width can be counted at all;
        // the value itself is carried alongside, because the answer this is
        // compared against is a list of keys at the row's own item type.
        let mut exact: HashMap<K::CountKey, (K, u64)> = HashMap::new();
        for it in items {
            exact
                .entry(it.count_key())
                .or_insert_with(|| (it.clone(), 0))
                .1 += 1;
        }
        let stream = StreamStats::of_counts(exact.values().map(|(_, c)| *c));
        let mut exact_vec: Vec<&(K, u64)> = exact.values().collect();
        // Count descending, ties on the key — a total order, the same one every
        // top-k impl ranks by. On count alone the k-th place falls to HashMap
        // order, and an exact source scores below 1.0 against itself.
        exact_vec.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.count_key().cmp(&b.0.count_key()))
        });
        exact_vec.truncate(self.k);
        let top = exact_vec.into_iter().cloned().collect();
        Ok(TopkTruth { top, exact, stream })
    }

    fn probes(&self, _truth: &TopkTruth<K>) -> Vec<()> {
        vec![(); QUERY_TIMING_REPEATS]
    }

    fn score(
        &self,
        truth: &TopkTruth<K>,
        _probes: &[()],
        answers: &[Vec<(K, u64)>],
    ) -> BTreeMap<String, f64> {
        let TopkTruth {
            top: truth,
            exact,
            stream,
        } = truth;
        // Every answer is to the same question, so the first is the estimate
        // and the rest existed to make the timing readable.
        let empty = Vec::new();
        let est = answers.first().unwrap_or(&empty);
        let est_set: std::collections::HashSet<K::CountKey> =
            est.iter().map(|(k, _)| k.count_key()).collect();
        let truth_set: std::collections::HashSet<K::CountKey> =
            truth.iter().map(|(k, _)| k.count_key()).collect();

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

        let count_errs: Vec<f64> = est
            .iter()
            .map(|(k, c)| {
                let true_count = exact.get(&k.count_key()).map_or(0, |(_, c)| *c);
                (*c as f64 - true_count as f64).abs()
            })
            .collect();
        let max_count_err = count_errs.iter().copied().fold(0.0f64, f64::max);
        let mean_count_err = if count_errs.is_empty() {
            0.0
        } else {
            count_errs.iter().sum::<f64>() / count_errs.len() as f64
        };

        let mut metrics: BTreeMap<String, f64> = [
            ("k".to_string(), self.k as f64),
            ("precision_at_k".to_string(), precision),
            ("recall_at_k".to_string(), recall),
            ("true_top_k_count".to_string(), truth_set.len() as f64),
            ("est_top_k_count".to_string(), est.len() as f64),
            ("max_count_err".to_string(), max_count_err),
            ("mean_count_err".to_string(), mean_count_err),
        ]
        .into_iter()
        .collect();
        stream.write(&mut metrics);
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;
    use std::collections::BinaryHeap;

    #[test]
    fn reported_counts_are_scored_against_the_exact_counts() {
        let items = tied_at_the_boundary();
        let table = table_of(&["key"], vec![ColumnData::Int64(items.clone())]);
        let gt = TopkGT::<i64>::over_column(3, 0);
        let off_by_two = |_: &mut ExactTopK, _: &()| vec![(1, 12), (2, 5), (99, 1)];
        let cmp = crate::accuracy::score_with(&gt, &off_by_two, &mut exact_source(&items), &table);
        assert_eq!(cmp["max_count_err"], 2.0);
        assert_eq!(cmp["mean_count_err"], 1.0);
        assert_eq!(cmp["stream_items"], 35.0);
        assert_eq!(cmp["stream_distinct"], 6.0);
        assert!((cmp["stream_l2"] - (100.0f64 + 5.0 * 25.0).sqrt()).abs() < 1e-12);
    }

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
        let mut items: Vec<i64> = std::iter::repeat_n(1, 10).collect();
        for key in 2..=6 {
            items.extend(std::iter::repeat_n(key, 5));
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
    fn query_exact(s: &mut ExactTopK, _: &()) -> Vec<(i64, u64)> {
        s.estimate_topk(3)
    }

    /// The exact baseline is this comparator's own check: anything below 1.0
    /// means the comparator is wrong, not the sketch. Repeated because the
    /// failure it guards was a per-`HashMap`-seed coin flip.
    #[test]
    fn an_exact_source_scores_precision_and_recall_of_one() {
        let items = tied_at_the_boundary();
        let table = table_of(&["key"], vec![ColumnData::Int64(items.clone())]);
        let gt = TopkGT::<i64>::over_column(3, 0);
        for _ in 0..32 {
            let cmp =
                crate::accuracy::score_with(&gt, &query_exact, &mut exact_source(&items), &table);
            assert_eq!(cmp["precision_at_k"], 1.0, "{:?}", cmp);
            assert_eq!(cmp["recall_at_k"], 1.0, "{:?}", cmp);
        }
    }

    /// Scoring is a pure function of `(sketch, items, k)`.
    #[test]
    fn scoring_the_same_sketch_twice_gives_the_same_answer() {
        let items = tied_at_the_boundary();
        let sketch = exact_source(&items);
        let table = table_of(&["key"], vec![ColumnData::Int64(items)]);
        let gt = TopkGT::<i64>::over_column(3, 0);
        let first = crate::accuracy::score_with(&gt, &query_exact, &mut sketch.clone(), &table);
        for _ in 0..32 {
            assert_eq!(
                crate::accuracy::score_with(&gt, &query_exact, &mut sketch.clone(), &table),
                first
            );
        }
    }
}
