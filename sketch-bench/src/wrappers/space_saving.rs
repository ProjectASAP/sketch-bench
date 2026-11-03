use crate::params::{ParamSet, SpaceSavingParams};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::SpaceSaving;

pub struct SpaceSavingLib {
    inner: SpaceSaving,
    k: usize,
}

impl<T: FrequencyValue> RowSketch<T> for SpaceSavingLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: SpaceSavingParams = params.parse()?;
        require_positive("asap SpaceSaving", "capacity", p.capacity)?;
        Ok(SpaceSavingLib {
            inner: SpaceSaving::with_capacity(p.capacity),
            k: topk_k("asap SpaceSaving", p.topk_k, p.capacity)?,
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner.update(&item.data_input());
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl<T: FrequencyValue> Answers<T, u64> for SpaceSavingLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner.estimate(&probe.data_input())
    }
}

impl<T: FrequencyValue> Answers<(), TopkAnswer<T>> for SpaceSavingLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<T> {
        let entries = self
            .inner
            .top_k(self.k)
            .into_iter()
            .map(|(key, count, _)| (T::from_data_input(&key.as_data_input()), count))
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for SpaceSavingLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
