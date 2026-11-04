use crate::params::{BloomParams, ParamSet};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{
    partition, require_positive, require_range, BuildError, Pass, QueryPass, Shared, StepPass,
};
use asap_sketchlib::sketches::bloom::{BLOOM_MAX_BITS, BLOOM_MAX_SLICES};
use asap_sketchlib::Bloom;
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap Bloom";

pub struct BloomLib {
    inner: Bloom,
}

pub fn build_bloom_lib(config: &ParamSet) -> Result<BloomLib, BuildError> {
    let p: BloomParams = config.parse()?;
    require_positive(NAME, "cols", p.cols)?;
    require_range(NAME, "rows", p.rows, 1, BLOOM_MAX_SLICES)?;
    if !p.cols.is_power_of_two() {
        return Err(BuildError(format!(
            "{NAME}: cols={} is not a power of two, where the library's false-positive bound holds",
            p.cols
        )));
    }
    if p.rows.saturating_mul(p.cols) > BLOOM_MAX_BITS {
        return Err(BuildError(format!(
            "{NAME}: rows={} x cols={} exceeds the {BLOOM_MAX_BITS} bits a filter holds",
            p.rows, p.cols
        )));
    }
    Ok(BloomLib {
        inner: Bloom::with_dimensions(p.rows, p.cols),
    })
}

pub fn memory_bloom_lib(sketch: &BloomLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_bloom_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.data_input());
            }
            memory_bloom_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_bloom_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.data_input());
            }),
            footprint: Box::new(move || memory_bloom_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<bool>>, BuildError> {
    merge_query_bloom_lib(params, items, probes, 1, passes)
}

pub fn merge_query_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<bool>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = bloom_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.contains(&p.data_input()));
            }
            let footprint = memory_bloom_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<bool>);
    }
    Ok(out)
}

pub fn merge_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = bloom_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_bloom_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_bloom_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = bloom_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_bloom_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn bloom_lib_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(BloomLib, Vec<BloomLib>), BuildError> {
    let mut parts: Vec<BloomLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_bloom_lib(params)?;
        for v in shard {
            sketch.inner.update(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_power_of_two_width_builds() {
        let ok = ParamSet::single("bloom", "rows=4 cols=65536").unwrap();
        assert!(build_bloom_lib(&ok).is_ok());
        let odd = ParamSet::single("bloom", "rows=4 cols=60000").unwrap();
        assert!(build_bloom_lib(&odd).is_err());
    }
}
