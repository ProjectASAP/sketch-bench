use crate::params::{CountMinHllParams, ParamSet};
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::hydra_shared::labels;
use crate::wrappers::polars_shared::{group_key, write_subset_key};
use crate::wrappers::{
    partition, require_positive, require_range, BuildError, Pass, QueryPass, Shared, StepPass,
};
use aqpbm_core::accuracy::subpopulation::GroupKey;
use asap_sketchlib::{CountMinHll, DataInput};
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap CountMinHll";

pub struct CountMinHllLib {
    inner: CountMinHll,
    key: String,
}

pub fn build_count_min_hll_lib(config: &ParamSet) -> Result<CountMinHllLib, BuildError> {
    let p: CountMinHllParams = config.parse()?;
    require_positive(NAME, "rows", p.rows)?;
    require_range(NAME, "precision", p.precision, 1, 18)?;
    if p.cols < 2 || !p.cols.is_power_of_two() {
        return Err(BuildError(format!(
            "{NAME}: cols={} must be a power of two and at least 2",
            p.cols
        )));
    }
    if p.rows * p.cols.trailing_zeros() as usize > 128 {
        return Err(BuildError(format!(
            "{NAME}: rows={} x {} column bits exceed the 128-bit hash",
            p.rows,
            p.cols.trailing_zeros()
        )));
    }
    Ok(CountMinHllLib {
        inner: CountMinHll::with_dimensions(p.rows, p.cols, p.precision),
        key: String::new(),
    })
}

pub fn memory_count_min_hll_lib(sketch: &CountMinHllLib) -> usize {
    sketch.inner.memory_bytes()
}

impl CountMinHllLib {
    fn update<V: CardinalityValue>(&mut self, record: &(String, V)) {
        let parts: Vec<&str> = record.0.split(';').collect();
        let value = record.1.data_input();
        for mask in 1..(1usize << parts.len()) {
            write_subset_key(&parts, mask, &mut self.key);
            self.inner.update(&DataInput::Str(&self.key), &value);
        }
    }

    fn estimate(&self, group: &GroupKey) -> f64 {
        let key = group_key(&labels(group));
        self.inner.estimate(&DataInput::Str(&key))
    }

    fn merge(&mut self, other: &Self) {
        self.inner
            .merge(&other.inner)
            .unwrap_or_else(|e| panic!("{NAME}: shards built alike refused to merge: {e}"));
    }
}

pub fn insert_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_count_min_hll_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.update(v);
            }
            memory_count_min_hll_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_count_min_hll_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.update(v);
            }),
            footprint: Box::new(move || memory_count_min_hll_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<GroupKey>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_count_min_hll_lib(params, items, probes, 1, passes)
}

pub fn merge_query_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<GroupKey>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = count_min_hll_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.merge(other);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate(p));
            }
            let footprint = memory_count_min_hll_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = count_min_hll_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.merge(other);
            }
            memory_count_min_hll_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_count_min_hll_lib<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = count_min_hll_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.merge(other);
            }),
            footprint: Box::new(move || memory_count_min_hll_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn count_min_hll_lib_shards<V: CardinalityValue>(
    params: &ParamSet,
    items: &[(String, V)],
    shards: usize,
) -> Result<(CountMinHllLib, Vec<CountMinHllLib>), BuildError> {
    let mut parts: Vec<CountMinHllLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_count_min_hll_lib(params)?;
        for v in shard {
            sketch.update(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
