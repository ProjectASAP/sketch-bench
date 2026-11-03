use crate::params::{CocoParams, ParamSet};
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::CocoSketch;

pub struct CocoLib {
    inner: CocoSketch,
}

impl RowSketch<String> for CocoLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CocoParams = params.parse()?;
        require_positive("asap CocoSketch", "rows", p.rows)?;
        require_positive("asap CocoSketch", "cols", p.cols)?;
        Ok(CocoLib {
            inner: CocoSketch::with_dimensions_and_seed(p.rows, p.cols, p.seed),
        })
    }

    fn insert(&mut self, item: &String) {
        self.inner.update_weighted(item, 1);
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<String, u64> for CocoLib {
    fn answer(&mut self, probe: &String) -> u64 {
        self.inner.estimate(probe)
    }
}

impl Merges for CocoLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
