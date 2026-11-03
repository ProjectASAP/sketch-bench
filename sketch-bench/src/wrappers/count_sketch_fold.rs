use crate::params::{CountSketchFoldParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::count_min_fold::check_fold;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::CountSketchFold;

pub struct CountSketchFoldLib {
    inner: CountSketchFold,
    k: usize,
}

impl<T: FrequencyValue> RowSketch<T> for CountSketchFoldLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchFoldParams = params.parse()?;
        let name = "asap CountSketchFold";
        check_fold(name, p.rows, p.full_cols, p.fold_level)?;
        require_positive(name, "heap_size", p.heap_size)?;
        Ok(CountSketchFoldLib {
            inner: CountSketchFold::new(p.rows, p.full_cols, p.fold_level, p.heap_size),
            k: topk_k(name, p.topk_k, p.heap_size)?,
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner.update(&item.data_input());
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl<T: FrequencyValue> Answers<T, u64> for CountSketchFoldLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner.estimate(&probe.data_input()).max(0) as u64
    }
}

impl<T: FrequencyValue> Answers<(), TopkAnswer<T>> for CountSketchFoldLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<T> {
        let entries = self
            .inner
            .heap()
            .heap()
            .iter()
            .map(|e| {
                (
                    T::from_data_input(&e.key.as_data_input()),
                    e.count.max(0) as u64,
                )
            })
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for CountSketchFoldLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge_same_level(&other.inner);
    }
}
