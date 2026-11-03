use crate::params::{ParamSet, UniformSamplingParams};
use crate::wrappers::quantile_value::QuantileValue;
use crate::wrappers::row_bodies::{Answers, Merges, RowSketch};
use crate::wrappers::BuildError;
use asap_sketchlib::UniformSampling;

pub struct UniformSamplingLib {
    inner: UniformSampling,
    sorted: Option<Vec<f64>>,
}

impl<T: QuantileValue + 'static> RowSketch<T> for UniformSamplingLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: UniformSamplingParams = params.parse()?;
        if !(p.sample_rate > 0.0 && p.sample_rate <= 1.0) {
            return Err(BuildError(format!(
                "asap UniformSampling: sample_rate={} outside (0, 1]",
                p.sample_rate
            )));
        }
        Ok(UniformSamplingLib {
            inner: UniformSampling::with_seed(p.sample_rate, p.seed),
            sorted: None,
        })
    }

    fn insert(&mut self, item: &T) {
        self.inner
            .update(crate::wrappers::quantile_value::ToF64::to_f64(*item));
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<f64, f64> for UniformSamplingLib {
    fn answer(&mut self, probe: &f64) -> f64 {
        let inner = &self.inner;
        let sorted = self.sorted.get_or_insert_with(|| {
            let mut samples = inner.samples();
            samples.sort_by(f64::total_cmp);
            samples
        });
        if sorted.is_empty() {
            return f64::NAN;
        }
        let rank = (probe * sorted.len() as f64).ceil() as usize;
        sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
    }
}

impl Merges for UniformSamplingLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner).unwrap_or_else(|e| {
            panic!("asap UniformSampling: shards built alike refused to merge: {e}")
        });
        self.sorted = None;
    }
}
