use crate::params::{ParamSet, UnivMonTwoTierParams};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::UnivMonTwoTier;
use std::marker::PhantomData;

pub struct Cardinality;
pub struct L1Norm;
pub struct L2Norm;
pub struct Entropy;

pub struct UnivMonTwoTierLib<Q> {
    inner: UnivMonTwoTier,
    statistic: PhantomData<Q>,
}

impl<K: FrequencyValue, Q: 'static> RowSketch<(K, i64)> for UnivMonTwoTierLib<Q> {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: UnivMonTwoTierParams = params.parse()?;
        let name = "asap UnivMonTwoTier";
        for (knob, value) in [
            ("heap_size", p.heap_size),
            ("elephant_rows", p.elephant_rows),
            ("elephant_cols", p.elephant_cols),
            ("mouse_rows", p.mouse_rows),
            ("mouse_cols", p.mouse_cols),
            ("layers", p.layers),
        ] {
            require_positive(name, knob, value)?;
        }
        if p.elephant_layers > p.layers {
            return Err(BuildError(format!(
                "{name}: elephant_layers={} exceeds layers={}",
                p.elephant_layers, p.layers
            )));
        }
        Ok(UnivMonTwoTierLib {
            inner: UnivMonTwoTier::new(
                p.heap_size,
                p.elephant_layers,
                p.elephant_rows,
                p.elephant_cols,
                p.mouse_rows,
                p.mouse_cols,
                p.layers,
            ),
            statistic: PhantomData,
        })
    }

    fn insert(&mut self, item: &(K, i64)) {
        self.inner.update_weighted(&item.0.data_input(), item.1);
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<(), f64> for UnivMonTwoTierLib<Cardinality> {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate_cardinality()
    }
}

impl Answers<(), f64> for UnivMonTwoTierLib<L1Norm> {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate_l1()
    }
}

impl Answers<(), f64> for UnivMonTwoTierLib<L2Norm> {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate_l2()
    }
}

impl Answers<(), f64> for UnivMonTwoTierLib<Entropy> {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate_entropy()
    }
}

impl<Q> Merges for UnivMonTwoTierLib<Q> {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
