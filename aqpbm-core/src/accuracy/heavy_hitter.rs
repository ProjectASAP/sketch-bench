//! Heavy-hitter ground truth: the exact heavy set from a `HashMap` counter,
//! scored on precision, recall and the weights. "Heavy" is a line someone draws
//! — `count > phi * n` — and the row's `ask` has to draw it the same way.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable};

use super::GroundTruth;

/// Comparator for a sketch that reports the heavy items and their weights.
pub struct HeavyHitterGT<K> {
    /// The heaviness bound, as a fraction of the stream: a key is heavy when it
    /// occurs more than `phi * n` times.
    pub phi: f64,
    pub column: usize,
    key: PhantomData<K>,
}

impl<K> HeavyHitterGT<K> {
    pub fn over_column(phi: f64, column: usize) -> Self {
        Self {
            phi,
            column,
            key: PhantomData,
        }
    }
}

impl<K> GroundTruth for HeavyHitterGT<K>
where
    K: ColumnItem + Eq + Hash + Ord,
{
    /// The true heavy keys with their exact counts, heaviest first.
    type Truth = Vec<(K, u64)>;
    /// One question: the whole set. A probe carries nothing.
    type Probe = ();
    type Answer = Vec<(K, u64)>;

    fn truth(&self, table: &GeneratedTable) -> Result<Vec<(K, u64)>, DataGenError> {
        let items = K::column_slice(table.column(self.column)?)?;
        let mut exact: HashMap<&K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it).or_insert(0) += 1;
        }
        let bound = self.phi * items.len() as f64;
        let mut heavy: Vec<(K, u64)> = exact
            .into_iter()
            .filter(|(_, c)| *c as f64 > bound)
            .map(|(k, c)| (k.clone(), c))
            .collect();
        // Count descending, ties on the key — a total order, so the set is
        // reported in one order regardless of `HashMap` seed.
        heavy.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(heavy)
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

        let est_set: HashSet<&K> = est.iter().map(|(k, _)| k).collect();
        let truth_set: HashSet<&K> = truth.iter().map(|(k, _)| k).collect();
        let tp = est_set.intersection(&truth_set).count() as f64;

        // An estimator naming nothing has no precision to speak of, and one
        // scored against an empty truth has no recall. Both report 0.0 rather
        // than dividing by zero — a null estimator must not score 1.0.
        let precision = if est_set.is_empty() {
            0.0
        } else {
            tp / est_set.len() as f64
        };
        let recall = if truth_set.is_empty() {
            0.0
        } else {
            tp / truth_set.len() as f64
        };

        // "and how heavy": the weight error over the keys both agree are heavy.
        // Reported over the intersection only, so a miss is counted by recall
        // and never twice.
        let exact: HashMap<&K, u64> = truth.iter().map(|(k, c)| (k, *c)).collect();
        let mut are = 0.0f64;
        let mut counted = 0usize;
        for (key, weight) in est {
            if let Some(t) = exact.get(key) {
                if *t > 0 {
                    are += (*weight as f64 - *t as f64).abs() / *t as f64;
                    counted += 1;
                }
            }
        }

        BTreeMap::from([
            ("phi".to_string(), self.phi),
            ("precision".to_string(), precision),
            ("recall".to_string(), recall),
            (
                "are_heavy".to_string(),
                if counted > 0 {
                    are / counted as f64
                } else {
                    0.0
                },
            ),
            ("true_heavy_count".to_string(), truth_set.len() as f64),
            ("est_heavy_count".to_string(), est_set.len() as f64),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;
    use aqpbm_datagen::{ColumnData, GeneratedTable};

    fn one_column(items: &[i64]) -> GeneratedTable {
        table_of(&["key"], vec![ColumnData::Int64(items.to_vec())])
    }

    /// An exact `HashMap` counter, which is what the comparator's own truth is
    /// built from — so it must score 1.0 on both axes. Anything less is the
    /// comparator being wrong, not the sketch.
    #[derive(Default, Clone)]
    struct ExactHeavy {
        counts: HashMap<i64, u64>,
    }

    impl ExactHeavy {
        fn update(&mut self, v: &i64) {
            *self.counts.entry(*v).or_insert(0) += 1;
        }
        fn heavy(&self, phi: f64, n: usize) -> Vec<(i64, u64)> {
            let bound = phi * n as f64;
            let mut out: Vec<(i64, u64)> = self
                .counts
                .iter()
                .filter(|(_, c)| **c as f64 > bound)
                .map(|(k, c)| (*k, *c))
                .collect();
            out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            out
        }
    }

    /// Keys 1..=3 are heavy at phi = 0.1; keys 4..=20 each occur once and are
    /// not. The boundary is what the tests are about, so the counts straddle it.
    fn skewed() -> Vec<i64> {
        let mut items: Vec<i64> = Vec::new();
        for key in 1..=3 {
            items.extend(std::iter::repeat_n(key, 30));
        }
        items.extend(4..=20);
        items
    }

    const PHI: f64 = 0.1;

    fn exact_source(items: &[i64]) -> ExactHeavy {
        let mut s = ExactHeavy::default();
        for it in items {
            s.update(it);
        }
        s
    }

    #[test]
    fn an_exact_source_scores_precision_and_recall_of_one() {
        let items = skewed();
        let n = items.len();
        let gt = HeavyHitterGT::<i64>::over_column(PHI, 0);
        let ask = move |s: &mut ExactHeavy, _: &()| s.heavy(PHI, n);
        // Repeated: the truth is built from a `HashMap`, so a tie-break that
        // leaned on iteration order would be a per-seed coin flip.
        for _ in 0..32 {
            let m = crate::accuracy::score_with(
                &gt,
                &ask,
                &mut exact_source(&items),
                &one_column(&items),
            );
            assert_eq!(m["precision"], 1.0, "{m:?}");
            assert_eq!(m["recall"], 1.0, "{m:?}");
            assert_eq!(m["are_heavy"], 0.0, "{m:?}");
            assert_eq!(m["true_heavy_count"], 3.0, "{m:?}");
        }
    }

    /// An estimator naming nothing scores 0.0 on both axes — not 1.0, which a
    /// `0/0` would have produced.
    #[test]
    fn a_null_estimator_scores_zero() {
        let items = skewed();
        let gt = HeavyHitterGT::<i64>::over_column(PHI, 0);
        let ask = |_: &mut ExactHeavy, _: &()| Vec::new();
        let m =
            crate::accuracy::score_with(&gt, &ask, &mut exact_source(&items), &one_column(&items));
        assert_eq!(m["precision"], 0.0);
        assert_eq!(m["recall"], 0.0);
        assert_eq!(m["est_heavy_count"], 0.0);
        assert_eq!(m["true_heavy_count"], 3.0);
    }

    /// The bound is strict (`>`), and it is a fraction of the stream: at
    /// phi = 0.3 exactly one third of a 90-item stream is *not* heavy.
    #[test]
    fn the_bound_is_strictly_greater_and_scales_with_the_stream() {
        let items: Vec<i64> = (1..=3).flat_map(|k| std::iter::repeat_n(k, 30)).collect();
        let table = one_column(&items);
        let gt = HeavyHitterGT::<i64>::over_column(0.3, 0);
        let truth = gt.truth(&table).unwrap();
        assert_eq!(truth.len(), 3, "30 > 0.3*90 = 27");

        let gt = HeavyHitterGT::<i64>::over_column(1.0 / 3.0, 0);
        let truth = gt.truth(&table).unwrap();
        assert!(truth.is_empty(), "30 is not strictly greater than 30");
    }

    /// Weight error is reported over the keys both sides agree on, so an
    /// over-count shows up in `are_heavy` and not in precision.
    #[test]
    fn overcounted_weights_land_in_are_heavy() {
        let items = skewed();
        let n = items.len();
        let gt = HeavyHitterGT::<i64>::over_column(PHI, 0);
        // Every weight doubled: the set is right, the weights are 100% off.
        let ask = move |s: &mut ExactHeavy, _: &()| {
            s.heavy(PHI, n)
                .into_iter()
                .map(|(k, c)| (k, c * 2))
                .collect()
        };
        let m =
            crate::accuracy::score_with(&gt, &ask, &mut exact_source(&items), &one_column(&items));
        assert_eq!(m["precision"], 1.0);
        assert_eq!(m["recall"], 1.0);
        assert!((m["are_heavy"] - 1.0).abs() < 1e-12, "{m:?}");
    }
}
