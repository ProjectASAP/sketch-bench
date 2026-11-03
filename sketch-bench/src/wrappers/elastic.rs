use crate::params::{ElasticParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::row_bodies::{ranked, topk_k, Answers, Merges, RowSketch};
use crate::wrappers::{require_positive, BuildError};
use asap_sketchlib::{CountMinSketch, ElasticSketch};

pub struct ElasticLib {
    inner: ElasticSketch,
    k: usize,
}

impl RowSketch<String> for ElasticLib {
    fn build(params: &ParamSet) -> Result<Self, BuildError> {
        let p: ElasticParams = params.parse()?;
        require_positive("asap ElasticSketch", "heavy_buckets", p.heavy_buckets)?;
        let buckets = i32::try_from(p.heavy_buckets).map_err(|_| {
            BuildError(format!(
                "asap ElasticSketch: heavy_buckets={} exceeds the i32 its API takes",
                p.heavy_buckets
            ))
        })?;
        CountMinSketch::check_shape(p.light_rows, p.light_cols)
            .map_err(|e| BuildError(format!("asap ElasticSketch light layer: {e}")))?;
        Ok(ElasticLib {
            inner: ElasticSketch::with_heavy_and_light(buckets, p.light_rows, p.light_cols),
            k: topk_k("asap ElasticSketch", p.topk_k, p.heavy_buckets)?,
        })
    }

    fn insert(&mut self, item: &String) {
        self.inner.update(item.clone());
    }

    fn memory(&self) -> usize {
        self.inner.memory_bytes()
    }
}

impl Answers<String, u64> for ElasticLib {
    fn answer(&mut self, probe: &String) -> u64 {
        self.inner.estimate(probe.clone()).max(0) as u64
    }
}

impl Answers<(), TopkAnswer<String>> for ElasticLib {
    fn answer(&mut self, _probe: &()) -> TopkAnswer<String> {
        let entries = self
            .inner
            .top_k(self.k)
            .into_iter()
            .map(|(key, count)| (key, count.max(0) as u64))
            .collect();
        ranked(entries, self.k)
    }
}

impl Merges for ElasticLib {
    fn merge(&mut self, other: &Self) {
        self.inner.merge(&other.inner);
    }
}
