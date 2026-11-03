use crate::params::{CountSketchSpaceSavingParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::CountSketchWithSpaceSaving;

pub struct CountSketchSpaceSavingLib {
    inner: CountSketchWithSpaceSaving,
    k: usize,
}

impl<T: FrequencyValue> RowSketch<T> for CountSketchSpaceSavingLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchSpaceSavingParams = params.parse()?;
        let name = "asap CountSketchWithSpaceSaving";
        require_positive(name, "capacity", p.capacity)?;
        CountSketchWithSpaceSaving::check_shape(p.rows, p.cols)
            .map_err(|e| BuildError(format!("{name}: {e}")))?;
        Ok(CountSketchSpaceSavingLib {
            inner: CountSketchWithSpaceSaving::new(p.rows, p.cols, p.capacity),
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

impl<T: FrequencyValue> Answers<T, u64> for CountSketchSpaceSavingLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner.estimate(&probe.data_input()).round().max(0.0) as u64
    }
}

impl<T: FrequencyValue> Answers<(), TopkAnswer<T>> for CountSketchSpaceSavingLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<T> {
        let entries = self
            .inner
            .top_k(self.k)
            .into_iter()
            .map(|(key, count)| {
                (
                    T::from_data_input(&key.as_data_input()),
                    count.round().max(0.0) as u64,
                )
            })
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for CountSketchSpaceSavingLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
