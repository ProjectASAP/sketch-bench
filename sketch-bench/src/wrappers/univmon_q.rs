use crate::params::{ParamSet, UnivMonQParams};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::quantile_value::QuantileValue;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::BuildError;
use asap_sketchlib::common::numerical::NumericalValue;
use asap_sketchlib::{UnivMonQ, UnivMonQConfig};

pub trait UnivMonQValue: NumericalValue + QuantileValue + 'static {
    fn from_f64(value: f64) -> Self;
}

impl UnivMonQValue for i64 {
    fn from_f64(value: f64) -> Self {
        value as i64
    }
}

impl UnivMonQValue for u64 {
    fn from_f64(value: f64) -> Self {
        value as u64
    }
}

impl UnivMonQValue for f64 {
    fn from_f64(value: f64) -> Self {
        value
    }
}

pub struct UnivMonQLib {
    inner: UnivMonQ,
    k: usize,
}

impl<T: UnivMonQValue> RowSketch<T> for UnivMonQLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: UnivMonQParams = params.parse()?;
        let config = UnivMonQConfig {
            layers: p.layers,
            top_layer_cols: p.top_layer_cols,
            cols_halving_period: p.cols_halving_period,
            rows: p.rows,
            counter_bits: p.counter_bits,
            heap_size: p.heap_size,
            ordered_samples: p.ordered_samples,
            seed_index: p.seed_index,
        };
        let inner =
            UnivMonQ::new(config).map_err(|e| BuildError(format!("asap UnivMonQ: {e:?}")))?;
        Ok(UnivMonQLib {
            inner,
            k: topk_k("asap UnivMonQ", p.topk_k, p.heap_size)?,
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner.update(item);
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<f64, f64> for UnivMonQLib {
    fn answer(&mut self, probe: &f64) -> f64 {
        self.inner.quantile(*probe).unwrap_or(f64::NAN)
    }
}

impl Answers<(), f64> for UnivMonQLib {
    fn answer(&mut self, _probe: &()) -> f64 {
        self.inner.estimate_cardinality()
    }
}

impl<T: UnivMonQValue> Answers<T, u64> for UnivMonQLib {
    fn answer(&mut self, probe: &T) -> u64 {
        self.inner
            .estimate_frequency(NumericalValue::to_f64(*probe))
    }
}

impl<T: UnivMonQValue> Answers<(), TopkAnswer<T>> for UnivMonQLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<T> {
        let entries = self
            .inner
            .heavy_hitters(self.k)
            .into_iter()
            .map(|(value, count)| (T::from_f64(value), count))
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for UnivMonQLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner).unwrap_or_else(|e| {
            panic!("asap UnivMonQ: shards built alike refused to merge: {e:?}")
        });
    }
}
