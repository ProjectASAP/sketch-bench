use std::collections::BTreeMap;
use std::collections::HashSet;
use std::marker::PhantomData;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{curve, CountedValue, GroundTruth};

const MEMBER_PROBES: usize = 10_000;
const NON_MEMBER_PROBES: usize = 100_000;

pub struct MembershipGT<K> {
    pub column: usize,
    key: PhantomData<K>,
}

impl<K> MembershipGT<K> {
    pub fn over_column(column: usize) -> Self {
        Self {
            column,
            key: PhantomData,
        }
    }
}

pub struct MembershipTruth<K> {
    members: Vec<K>,
    non_members: Vec<K>,
    distinct: usize,
}

impl<K: CountedValue> GroundTruth for MembershipGT<K> {
    type Truth = MembershipTruth<K>;
    type Probe = K;
    type Answer = bool;

    fn truth(&self, table: &GeneratedTable) -> Result<MembershipTruth<K>, DataGenError> {
        let items = K::column_slice(table.column(self.column)?)?;
        let mut seen: HashSet<K::CountKey> = HashSet::new();
        let mut distinct: Vec<(K, u64)> = Vec::new();
        for it in items {
            if seen.insert(it.count_key()) {
                distinct.push((it.clone(), 1));
            }
        }
        let mut members = curve::shuffled(&distinct);
        members.truncate(MEMBER_PROBES);
        let non_members = (0..)
            .map(K::absent_candidate)
            .filter(|k| !seen.contains(&k.count_key()))
            .take(NON_MEMBER_PROBES)
            .collect();
        Ok(MembershipTruth {
            members,
            non_members,
            distinct: seen.len(),
        })
    }

    fn probes(&self, truth: &MembershipTruth<K>) -> Vec<K> {
        truth
            .members
            .iter()
            .chain(&truth.non_members)
            .cloned()
            .collect()
    }

    fn score(
        &self,
        truth: &MembershipTruth<K>,
        _probes: &[K],
        answers: &[bool],
    ) -> BTreeMap<String, f64> {
        let (members, non_members) = answers.split_at(truth.members.len().min(answers.len()));
        let missed = members.iter().filter(|present| !**present).count();
        let false_positives = non_members.iter().filter(|present| **present).count();
        let rate = |hits: usize, of: usize| {
            if of == 0 {
                0.0
            } else {
                hits as f64 / of as f64
            }
        };
        BTreeMap::from([
            ("fpr".to_string(), rate(false_positives, non_members.len())),
            ("fnr".to_string(), rate(missed, members.len())),
            ("member_probes".to_string(), members.len() as f64),
            ("non_member_probes".to_string(), non_members.len() as f64),
            ("stream_distinct".to_string(), truth.distinct as f64),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;

    #[test]
    fn an_exact_set_has_no_false_answers_and_a_yes_to_everything_has_fpr_one() {
        let items: Vec<i64> = vec![1, 2, 2, 3, -1];
        let table = table_of(&["key"], vec![ColumnData::Int64(items.clone())]);
        let gt = MembershipGT::<i64>::over_column(0);
        let set: HashSet<i64> = items.into_iter().collect();
        let mut exact = set.clone();
        let cmp = crate::accuracy::score_with(
            &gt,
            &|s: &mut HashSet<i64>, k: &i64| s.contains(k),
            &mut exact,
            &table,
        );
        assert_eq!(cmp["fpr"], 0.0);
        assert_eq!(cmp["fnr"], 0.0);
        assert_eq!(cmp["member_probes"], 4.0);
        assert_eq!(cmp["non_member_probes"], NON_MEMBER_PROBES as f64);
        assert_eq!(cmp["stream_distinct"], 4.0);
        let cmp = crate::accuracy::score_with(
            &gt,
            &|_: &mut HashSet<i64>, _: &i64| true,
            &mut exact,
            &table,
        );
        assert_eq!(cmp["fpr"], 1.0);
    }
}
