use crate::params::{CountMinSpaceSavingParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::CountMinSketchWithSpaceSaving;

pub struct CountMinSpaceSavingLib {
    inner: CountMinSketchWithSpaceSaving,
    k: usize,
}

impl<T: FrequencyValue> RowSketch<T> for CountMinSpaceSavingLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountMinSpaceSavingParams = params.parse()?;
        let name = "asap CountMinSketchWithSpaceSaving";
        require_positive(name, "capacity", p.capacity)?;
        CountMinSketchWithSpaceSaving::check_shape(p.rows, p.cols)
            .map_err(|e| BuildError(format!("{name}: {e}")))?;
        Ok(CountMinSpaceSavingLib {
            inner: CountMinSketchWithSpaceSaving::new(p.rows, p.cols, p.capacity),
            k: topk_k(name, p.topk_k, p.capacity)?,
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner.update(&item.data_input());
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl<T: FrequencyValue> Answers<T, u64> for CountMinSpaceSavingLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner.estimate(&probe.data_input()).max(0) as u64
    }
}

impl<T: FrequencyValue> Answers<(), TopkAnswer<T>> for CountMinSpaceSavingLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<T> {
        let entries = self
            .inner
            .top_k(self.k)
            .into_iter()
            .map(|(key, count)| {
                (
                    T::from_data_input(&key.as_data_input()),
                    count.max(0) as u64,
                )
            })
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for CountMinSpaceSavingLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
