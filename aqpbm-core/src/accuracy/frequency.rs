//! Frequency-algorithm ground truth (CMS, CountSketch).
//!
//! Error ships as a curve over the true top-k (`are_top1`…`are_top1000`) plus
//! `are_all`: ARE over all distinct keys is dominated by singletons, where an
//! all-zero estimator scores 1.0 (SALSA, ICDE 2021). AAE weights heavy keys.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable};

use super::curve;
use super::GroundTruth;

#[derive(Debug, Clone, Copy)]
pub struct FrequencyGT<K> {
    pub column: usize,
    key: PhantomData<K>,
}

impl<K> FrequencyGT<K> {
    pub fn over_column(column: usize) -> Self {
        Self {
            column,
            key: PhantomData,
        }
    }
}

/// Everything the probe set and the scoring read: the exact counts, the true
/// ranking, and the unfiltered population the `*_all` keys come from.
pub struct FrequencyTruth<K> {
    exact: HashMap<K, u64>,
    /// Keys by true count descending, ties on the key, so a top-k prefix is
    /// deterministic across runs and implementations.
    ranked: Vec<K>,
    /// The `*_all` population: distinct keys shuffled then capped.
    all: Vec<K>,
}

impl<K> GroundTruth for FrequencyGT<K>
where
    K: ColumnItem + Eq + Hash + Ord,
{
    type Truth = FrequencyTruth<K>;
    type Probe = K;
    type Answer = u64;

    fn truth(&self, table: &GeneratedTable) -> Result<FrequencyTruth<K>, DataGenError> {
        let items = K::column_slice(table.column(self.column)?)?;
        let mut exact: HashMap<K, u64> = HashMap::new();
        for it in items {
            *exact.entry(it.clone()).or_insert(0) += 1;
        }
        let mut by_count: Vec<(K, u64)> = exact.iter().map(|(k, c)| (k.clone(), *c)).collect();
        by_count.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let all = curve::shuffled(&by_count);
        let ranked = by_count.into_iter().map(|(k, _)| k).collect();
        Ok(FrequencyTruth { exact, ranked, all })
    }

    /// Every key any population needs, asked once. The prefixes re-use the
    /// answers instead of re-querying: an estimate is deterministic given the
    /// sketch, so a second call would only measure a warm cache.
    fn probes(&self, truth: &FrequencyTruth<K>) -> Vec<K> {
        curve::union_of(&truth.all, &truth.ranked)
    }

    fn score(
        &self,
        truth: &FrequencyTruth<K>,
        probes: &[K],
        answers: &[u64],
    ) -> BTreeMap<String, f64> {
        let estimates: HashMap<&K, u64> =
            probes.iter().zip(answers).map(|(k, v)| (k, *v)).collect();
        let mut metrics: BTreeMap<String, f64> = BTreeMap::new();
        curve::error_curve(
            &truth.ranked,
            &truth.all,
            |k| {
                (
                    *estimates.get(k).unwrap_or(&0) as f64,
                    *truth.exact.get(k).unwrap_or(&0) as f64,
                )
            },
            &mut metrics,
        );
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;

    fn one_column(items: Vec<i64>) -> aqpbm_datagen::GeneratedTable {
        table_of(&["key"], vec![ColumnData::Int64(items)])
    }

    /// The estimator that does no work: every frequency is zero.
    struct NullFreq;

    /// How it is asked. A plain closure now — there is no trait left to
    /// declare membership through, which is exactly what the registry test
    /// `every_row_answers_its_capability` exists to compensate for.
    fn query_null(_: &mut NullFreq, _: &i64) -> u64 {
        0
    }

    /// The property that makes the top-k curve worth reporting: a null
    /// estimator scores exactly 1.0 on ARE, on every population. A real sketch
    /// ranked *above* 1.0 means the metric is measuring the wrong thing.
    #[test]
    fn null_estimator_scores_exactly_one_on_are() {
        let items: Vec<i64> = (0..2000).map(|i| (i % 97) as i64).collect();
        let gt = FrequencyGT::<i64>::over_column(0);
        let cmp = crate::accuracy::score_with(&gt, &query_null, &mut NullFreq, &one_column(items));
        for key in ["are_all", "are_top1", "are_top10"] {
            let v = cmp[key];
            assert!(
                (v - 1.0).abs() < 1e-12,
                "null estimator must score exactly 1.0 on {key}, got {v}"
            );
        }
        // And zero absolute error is impossible unless the data is empty:
        // AAE for the null estimator is the mean true count.
        assert!(cmp["aae_all"] > 0.0);
    }

    #[test]
    fn top_k_prefixes_follow_the_true_ranking() {
        // Key 1 appears 100x, key 2 50x, the rest once.
        let mut items: Vec<i64> = Vec::new();
        items.extend(std::iter::repeat_n(1, 100));
        items.extend(std::iter::repeat_n(2, 50));
        items.extend(3..=200);
        let gt = FrequencyGT::<i64>::over_column(0);
        let cmp = crate::accuracy::score_with(&gt, &query_null, &mut NullFreq, &one_column(items));
        // top1 is key 1, so AAE over it is exactly its true count.
        assert_eq!(cmp["aae_top1"], 100.0);
        assert_eq!(cmp["probes_top1"], 1.0);
        assert_eq!(cmp["probes_top10"], 10.0);
        // 200 distinct keys: top1000 must be omitted, not silently aliased
        // to `all`.
        assert!(!cmp.contains_key("are_top1000"));
    }
}
