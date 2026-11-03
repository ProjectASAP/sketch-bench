use crate::params::ParamSet;
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::{partition, BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

pub trait RowSketch<I>: Sized + 'static {
    fn build(params: &ParamSet) -> Result<Self, BuildError>;
    fn insert(&mut self, item: &I);
    fn memory(&self) -> usize;
}

pub trait Answers<P, A> {
    fn answer(&mut self, probe: &P) -> A;
}

pub trait Merges {
    fn merge(&mut self, other: &Self);
}

fn fed<S: RowSketch<I>, I>(params: &ParamSet, items: &[I]) -> Result<S, BuildError> {
    let mut sketch = S::build(params)?;
    for v in items {
        sketch.insert(v);
    }
    Ok(sketch)
}

pub fn insert<S: RowSketch<I>, I: 'static>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = S::build(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.insert(v);
            }
            sketch.memory()
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step<S: RowSketch<I>, I: 'static>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<S> = Rc::new(RefCell::new(S::build(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| driven.borrow_mut().insert(&stream[i])),
            footprint: Box::new(move || read.borrow().memory()),
        });
    }
    Ok(out)
}

fn asked<S, P, A>(mut sketch: S, probes: Rc<Vec<P>>, memory: fn(&S) -> usize) -> QueryPass<A>
where
    S: Answers<P, A> + 'static,
    P: 'static,
    A: 'static,
{
    Box::new(move || {
        let mut answers = Vec::with_capacity(probes.len());
        for p in probes.iter() {
            answers.push(sketch.answer(p));
        }
        let footprint = memory(&sketch);
        (answers, footprint)
    })
}

pub fn query<S, I, P, A>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    probes: Rc<Vec<P>>,
    passes: usize,
) -> Result<Vec<QueryPass<A>>, BuildError>
where
    S: RowSketch<I> + Answers<P, A>,
    I: 'static,
    P: 'static,
    A: 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch = fed::<S, I>(params, &items)?;
        out.push(asked(sketch, probes.clone(), S::memory));
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn shards<S: RowSketch<I>, I>(
    params: &ParamSet,
    items: &[I],
    shards: usize,
) -> Result<(S, Vec<S>), BuildError> {
    let mut parts: Vec<S> = Vec::new();
    for shard in partition(items, shards) {
        parts.push(fed::<S, I>(params, shard)?);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn merge_query<S, I, P, A>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    probes: Rc<Vec<P>>,
    shard_count: usize,
    passes: usize,
) -> Result<Vec<QueryPass<A>>, BuildError>
where
    S: RowSketch<I> + Answers<P, A> + Merges,
    I: 'static,
    P: 'static,
    A: 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = shards::<S, I>(params, &items, shard_count)?;
        for other in rest.iter() {
            sketch.merge(other);
        }
        out.push(asked(sketch, probes.clone(), S::memory));
    }
    Ok(out)
}

pub fn merge<S: RowSketch<I> + Merges, I: 'static>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    shard_count: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = shards::<S, I>(params, &items, shard_count)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.merge(other);
            }
            acc.memory()
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step<S: RowSketch<I> + Merges, I: 'static>(
    params: &ParamSet,
    items: Rc<Vec<I>>,
    shard_count: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = shards::<S, I>(params, &items, shard_count)?;
        let acc: Shared<S> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| driven.borrow_mut().merge(&rest[i])),
            footprint: Box::new(move || read.borrow().memory()),
        });
    }
    Ok(out)
}

pub fn ranked<T>(mut entries: Vec<(T, u64)>, k: usize) -> TopkAnswer<T> {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    entries.truncate(k);
    entries
}

pub fn topk_k(name: &str, topk_k: Option<usize>, held: usize) -> Result<usize, BuildError> {
    let k = topk_k.unwrap_or(crate::wrappers::cms_heap::sketchlib::TOPK_K);
    if k == 0 {
        return Err(BuildError(format!("{name}: topk_k must be positive")));
    }
    if held < k {
        return Err(BuildError(format!(
            "{name}: holds {held} candidates, fewer than the k={k} it answers"
        )));
    }
    Ok(k)
}
