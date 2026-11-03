use crate::params::{CountMinHllParams, ParamSet};
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::hydra_shared::labels;
use crate::wrappers::polars_shared::{group_key, subset_key};
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use aqpbm_core::accuracy::subpopulation::GroupKey;
use asap_sketchlib::{CountMinHll, DataInput};

const PACKED_HASH_BITS: usize = 128;

pub struct CountMinHllLib {
    inner: CountMinHll,
}

impl<V: CardinalityValue> RowSketch<(String, V)> for CountMinHllLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: CountMinHllParams = params.parse()?;
        let name = "asap CountMinHll";
        require_positive(name, "rows", p.rows)?;
        crate::wrappers::require_range(name, "precision", p.precision, 1, 18)?;
        if p.cols < 2 || !p.cols.is_power_of_two() {
            return Err(BuildError(format!(
                "{name}: cols={} must be a power of two and at least 2",
                p.cols
            )));
        }
        let col_bits = p.cols.trailing_zeros() as usize;
        if p.rows * col_bits > PACKED_HASH_BITS {
            return Err(BuildError(format!(
                "{name}: rows={} x {col_bits} column bits exceed the {PACKED_HASH_BITS}-bit hash",
                p.rows
            )));
        }
        Ok(CountMinHllLib {
            inner: CountMinHll::with_dimensions(p.rows, p.cols, p.precision),
        })
    }

    fn insert(&mut self, item: &(String, V)) {
        let parts: Vec<&str> = item.0.split(';').collect();
        let value = item.1.data_input();
        for mask in 1..(1usize << parts.len()) {
            let key = subset_key(&parts, mask);
            self.inner.update(&DataInput::Str(&key), &value);
        }
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<GroupKey, f64> for CountMinHllLib {
    fn answer(&mut self, probe: &GroupKey) -> f64 {
        let key = group_key(&labels(probe));
        self.inner.estimate(&DataInput::Str(&key))
    }
}

impl Merges for CountMinHllLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner).unwrap_or_else(|e| {
            panic!("asap CountMinHll: shards built alike refused to merge: {e}")
        });
    }
}
