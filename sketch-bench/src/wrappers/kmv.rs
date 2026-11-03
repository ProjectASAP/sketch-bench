use crate::params::{KmvParams, ParamSet};
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::Kmv;

pub struct KmvLib {
    inner: Kmv,
}

impl<T: CardinalityValue> RowSketch<T> for KmvLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: KmvParams = params.parse()?;
        require_positive("asap Kmv", "k", p.k)?;
        Ok(KmvLib {
            inner: Kmv::new(p.k),
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner.update(&item.data_input());
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<(), f64> for KmvLib {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate()
    }
}

impl Merges for KmvLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
