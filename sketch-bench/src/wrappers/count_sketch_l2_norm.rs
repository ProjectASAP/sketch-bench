use crate::params::{CountSketchL2NormParams, ParamSet};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::BuildError;
use asap_sketchlib::{CountSketch, CountSketchWithL2Norm};

pub struct CountSketchL2NormLib {
    inner: CountSketchWithL2Norm,
}

impl<K: FrequencyValue> RowSketch<(K, i64)> for CountSketchL2NormLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchL2NormParams = params.parse()?;
        CountSketch::check_shape(p.rows, p.cols)
            .map_err(|e| BuildError(format!("asap CountSketchWithL2Norm: {e}")))?;
        Ok(CountSketchL2NormLib {
            inner: CountSketchWithL2Norm::with_dimensions(p.rows, p.cols),
        })
    }

    fn insert(&mut self, item: &(K, i64)) {
        self.inner.update_weighted(&item.0.data_input(), item.1);
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<(), f64> for CountSketchL2NormLib {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.l2()
    }
}

impl Merges for CountSketchL2NormLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
