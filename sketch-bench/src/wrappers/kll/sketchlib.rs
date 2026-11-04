//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

/// `k` this library cannot hold is an error naming both, not a run at some
/// other `k` reported as the one asked for.
fn lib_kll<T>(k: u32, seed: Option<u64>) -> Result<asap_sketchlib::Kll<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue,
{
    if !(LIB_K_MIN..=LIB_K_MAX).contains(&k) {
        return Err(BuildError(format!(
            "asap KLL: k={k} outside [{LIB_K_MIN}, {LIB_K_MAX}]; the library clamps \
             to that range, so any other k would run at a value this record does not name"
        )));
    }
    Ok(match seed {
        Some(seed) => asap_sketchlib::Kll::<T>::with_k_and_seed(k as i32, seed),
        None => asap_sketchlib::Kll::<T>::with_k(k as i32),
    })
}

/// Generic *in the library*: `KLL<T>` stores `T` and orders it with
/// `T::total_cmp`, converting nothing — so this row's item-type axis measures
/// the library's own choice, not a wrapper's cast.
pub struct KllLibPerCall<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::Kll<T>,
}

pub fn build_kll_lib_per_call<T>(config: &ParamSet) -> Result<KllLibPerCall<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibPerCall {
        inner: lib_kll::<T>(p.k, p.seed)?,
    })
}

pub fn memory_kll_lib_per_call<T>(sketch: &KllLibPerCall<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    sketch.inner.memory_bytes()
}

pub struct KllLibCdf<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::Kll<T>,
    cdf: Option<asap_sketchlib::sketches::kll::Cdf>,
}

pub fn build_kll_lib_cdf<T>(config: &ParamSet) -> Result<KllLibCdf<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibCdf {
        inner: lib_kll::<T>(p.k, p.seed)?,
        cdf: None,
    })
}

pub fn memory_kll_lib_cdf<T>(sketch: &KllLibCdf<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    sketch.inner.memory_bytes()
}

pub fn insert_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_lib_per_call::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_kll_lib_per_call::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_lib_per_call::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_kll_lib_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    merge_query_kll_lib_per_call::<T>(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = kll_lib_per_call_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p));
            }
            let footprint = memory_kll_lib_per_call::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_lib_per_call_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_kll_lib_per_call::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_lib_per_call_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_kll_lib_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_lib_per_call_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllLibPerCall<T>, Vec<KllLibPerCall<T>>), BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut parts: Vec<KllLibPerCall<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_lib_per_call::<T>(params)?;
        for v in shard {
            sketch.inner.update(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_kll_lib_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_lib_cdf::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_kll_lib_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    merge_query_kll_lib_cdf::<T>(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = kll_lib_cdf_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
            // A merged sketch invalidates any CDF cached from the pre-merge state.
            sketch.cdf = None;
        }
        let cdf = sketch.inner.cdf();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(cdf.value_at(*p));
            }
            let footprint = memory_kll_lib_cdf::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_lib_cdf_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
                // A merged sketch invalidates any CDF cached from the pre-merge state.
                acc.cdf = None;
            }
            memory_kll_lib_cdf::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_lib_cdf_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
                // A merged sketch invalidates any CDF cached from the pre-merge state.
                acc.cdf = None;
            }),
            footprint: Box::new(move || memory_kll_lib_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_lib_cdf_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllLibCdf<T>, Vec<KllLibCdf<T>>), BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut parts: Vec<KllLibCdf<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        for v in shard {
            sketch.inner.update(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn prepare_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        for v in items.iter() {
            sketch.inner.update(v);
        }
        out.push(Box::new(move || {
            sketch.cdf = Some(sketch.inner.cdf());
            memory_kll_lib_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn median_after(config: &str) -> f64 {
        let params = ParamSet::single("kll-percall", config).unwrap();
        let mut sketch = build_kll_lib_per_call::<i64>(&params).unwrap();
        for v in 0..100_000i64 {
            sketch.inner.update(&(v * 7919 % 100_003));
        }
        sketch.inner.quantile(0.5)
    }

    #[test]
    fn a_seed_fixes_the_compaction_and_oxide_refuses_one() {
        assert_eq!(median_after("k=50 seed=7"), median_after("k=50 seed=7"));
        let seeded = ParamSet::single("kll-percall", "k=200 seed=7").unwrap();
        assert!(crate::wrappers::kll::oxide::build_kll_oxide_per_call::<i64>(&seeded).is_err());
        let unseeded = ParamSet::single("kll-percall", "k=200").unwrap();
        assert!(crate::wrappers::kll::oxide::build_kll_oxide_per_call::<i64>(&unseeded).is_ok());
    }
}
