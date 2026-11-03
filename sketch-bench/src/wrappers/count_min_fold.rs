use crate::params::{CountMinFoldParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::CountMinSketchFold;

const MAX_FULL_COLS: usize = 65_536;

pub(crate) fn check_fold(
    name: &str,
    rows: usize,
    full_cols: usize,
    fold_level: u32,
) -> Result<(), BuildError> {
    require_positive(name, "rows", rows)?;
    if !full_cols.is_power_of_two() || full_cols > MAX_FULL_COLS {
        return Err(BuildError(format!(
            "{name}: full_cols={full_cols} must be a power of two at most {MAX_FULL_COLS}"
        )));
    }
    if fold_level > full_cols.trailing_zeros() {
        return Err(BuildError(format!(
            "{name}: fold_level={fold_level} folds full_cols={full_cols} below one column"
        )));
    }
    Ok(())
}

pub struct CountMinFoldLib {
    inner: CountMinSketchFold,
    k: usize,
}

impl<T: FrequencyValue> RowSketch<T> for CountMinFoldLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountMinFoldParams = params.parse()?;
        let name = "asap CountMinSketchFold";
        check_fold(name, p.rows, p.full_cols, p.fold_level)?;
        require_positive(name, "heap_size", p.heap_size)?;
        Ok(CountMinFoldLib {
            inner: CountMinSketchFold::new(p.rows, p.full_cols, p.fold_level, p.heap_size),
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

impl<T: FrequencyValue> Answers<T, u64> for CountMinFoldLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner.estimate(&probe.data_input()).max(0) as u64
    }
}

impl<T: FrequencyValue> Answers<(), TopkAnswer<T>> for CountMinFoldLib {
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

impl Merges for CountMinFoldLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge_same_level(&other.inner);
    }
}
