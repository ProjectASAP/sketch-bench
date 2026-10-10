//! The `asap_sketchlib` implementation.
//!
//! Grouped under `wrappers/hydra_cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{CountMin, FastPath, Hydra, Vector2D};
use std::cell::RefCell;
use std::rc::Rc;

pub struct HydraCms {
    pub(super) inner: Hydra,
    params: HydraCmsParams,
}

/// `label_columns`: the key columns every record carries.
pub fn build_hydra_cms(config: &ParamSet, label_columns: usize) -> Result<HydraCms, BuildError> {
    let p: HydraCmsParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-cms")?;
    for (name, v) in [("cell_rows", p.cell_rows), ("cell_cols", p.cell_cols)] {
        if v == 0 {
            return Err(BuildError(format!("hydra-cms: {name} must be > 0")));
        }
    }
    let cell = HydraCounter::CM(CountMin::<Vector2D<i32>, FastPath>::with_dimensions(
        p.cell_rows,
        p.cell_cols,
    ));
    Ok(HydraCms {
        inner: new_hydra(p.rows, p.cols, label_columns, cell, "hydra-cms")?,
        params: p,
    })
}

impl HydraCms {
    #[inline]
    pub fn estimate_subpop_frequency<V: FrequencyValue>(
        &self,
        labels: &[Option<&str>],
        value: &V,
    ) -> f64 {
        query(
            &self.inner,
            labels,
            &HydraQuery::Frequency(value.data_input()),
            "hydra-cms",
        )
    }
}

/// The grid holds `rows * cols` cells and every cell is a full Count-Min of
/// `i32` counters, so the counter term is the product of both shapes.
pub fn memory_hydra_cms(sketch: &HydraCms) -> usize {
    let p = &sketch.params;
    p.rows * p.cols * p.cell_rows * p.cell_cols * std::mem::size_of::<i32>()
        + grid_overhead_bytes(p.rows, p.cols)
}

pub fn insert_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_cms(params, label_columns(&items, "hydra-cms")?)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                update(&mut sketch.inner, &v.0, &v.1.data_input(), "hydra-cms");
            }
            memory_hydra_cms(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_cms(
            params,
            label_columns(&items, "hydra-cms")?,
        )?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                update(&mut sketch.inner, &v.0, &v.1.data_input(), "hydra-cms");
            }),
            footprint: Box::new(move || memory_hydra_cms(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<(Vec<Option<String>>, V)>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_hydra_cms(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<(Vec<Option<String>>, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = hydra_cms_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so grid and cell shapes match");
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_frequency(&labels(&p.0), &p.1));
            }
            let footprint = memory_hydra_cms(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_cms_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_cms(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_cms<V: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_cms_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }),
            footprint: Box::new(move || memory_hydra_cms(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_cms_shards<V: FrequencyValue>(
    params: &ParamSet,
    items: &[(String, V)],
    shards: usize,
) -> Result<(HydraCms, Vec<HydraCms>), BuildError> {
    let mut parts: Vec<HydraCms> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_cms(params, label_columns(items, "hydra-cms")?)?;
        for v in shard {
            update(&mut sketch.inner, &v.0, &v.1.data_input(), "hydra-cms");
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
