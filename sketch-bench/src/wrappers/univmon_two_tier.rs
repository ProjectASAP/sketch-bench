use crate::params::{ParamSet, UnivMonTwoTierParams};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::UnivMonTwoTier;
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap UnivMonTwoTier";

pub struct UnivMonTwoTierLib {
    inner: UnivMonTwoTier,
}

pub fn build_univmon_two_tier_lib(config: &ParamSet) -> Result<UnivMonTwoTierLib, BuildError> {
    let p: UnivMonTwoTierParams = config.parse()?;
    for (knob, value) in [
        ("heap_size", p.heap_size),
        ("elephant_rows", p.elephant_rows),
        ("elephant_cols", p.elephant_cols),
        ("mouse_rows", p.mouse_rows),
        ("mouse_cols", p.mouse_cols),
        ("layers", p.layers),
    ] {
        require_positive(NAME, knob, value)?;
    }
    if p.elephant_layers > p.layers {
        return Err(BuildError(format!(
            "{NAME}: elephant_layers={} exceeds layers={}",
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
    })
}

pub fn memory_univmon_two_tier_lib(sketch: &UnivMonTwoTierLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_univmon_two_tier_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_two_tier_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update_weighted(&v.0.data_input(), v.1);
            }
            memory_univmon_two_tier_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_univmon_two_tier_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_univmon_two_tier_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update_weighted(&v.0.data_input(), v.1);
            }),
            footprint: Box::new(move || memory_univmon_two_tier_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_univmon_two_tier_lib_cardinality<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_two_tier_lib_cardinality(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_two_tier_lib_cardinality<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate_cardinality());
            }
            let footprint = memory_univmon_two_tier_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_two_tier_lib_l1_norm<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_two_tier_lib_l1_norm(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_two_tier_lib_l1_norm<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate_l1());
            }
            let footprint = memory_univmon_two_tier_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_two_tier_lib_l2_norm<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_two_tier_lib_l2_norm(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_two_tier_lib_l2_norm<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate_l2());
            }
            let footprint = memory_univmon_two_tier_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_two_tier_lib_entropy<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_two_tier_lib_entropy(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_two_tier_lib_entropy<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate_entropy());
            }
            let footprint = memory_univmon_two_tier_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_univmon_two_tier_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_univmon_two_tier_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_univmon_two_tier_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = univmon_two_tier_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_univmon_two_tier_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn univmon_two_tier_lib_shards<K: FrequencyValue>(
    params: &ParamSet,
    items: &[(K, i64)],
    shards: usize,
) -> Result<(UnivMonTwoTierLib, Vec<UnivMonTwoTierLib>), BuildError> {
    let mut parts: Vec<UnivMonTwoTierLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_univmon_two_tier_lib(params)?;
        for v in shard {
            sketch.inner.update_weighted(&v.0.data_input(), v.1);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
