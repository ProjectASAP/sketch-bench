//! Exact multi-subpopulation accumulators for the `*_over_time` statistics,
//! matching ASAPQuery's `MultipleSumAccumulator`, `MultipleMinMaxAccumulator`
//! and `MultipleIncreaseAccumulator`
//! (`asap-query-engine/src/precompute_operators/`): a map from a group's
//! label values (`Vec<String>`, as `KeyByLabelValues`) to that group's state.
//! Written here, not taken from a library.
//!
//! The generated data has no timestamps, so a sample's row index in the
//! stream stands in for its timestamp (ms).

use crate::params::{ExactParams, ParamSet};
use crate::wrappers::quantile_value::ToF64;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use aqpbm_core::accuracy::aggregate::AggregateQueryError;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One group's state.
pub trait Cell: Clone + 'static {
    fn first(v: f64, ts: i64) -> Self;
    fn insert(&mut self, v: f64, ts: i64);
    fn merge(&mut self, other: &Self);
    fn value(&self) -> f64;
    /// Bytes this cell owns beyond `size_of::<Self>()`.
    fn heap_bytes(&self) -> usize {
        0
    }
}

/// `MultipleSumAccumulator`: one sum per group. Count is the same
/// accumulator fed 1 per sample.
#[derive(Clone)]
pub struct Sum(f64);

impl Cell for Sum {
    fn first(v: f64, _ts: i64) -> Self {
        Sum(v)
    }
    fn insert(&mut self, v: f64, _ts: i64) {
        self.0 += v;
    }
    fn merge(&mut self, other: &Self) {
        self.0 += other.0;
    }
    fn value(&self) -> f64 {
        self.0
    }
}

/// `MultipleMinMaxAccumulator` with `sub_type = "max"`. ASAPQuery picks min
/// or max with a flag on one type; min is the same cell flipped, same cost.
#[derive(Clone)]
pub struct Max(f64);

impl Cell for Max {
    fn first(v: f64, _ts: i64) -> Self {
        Max(v)
    }
    fn insert(&mut self, v: f64, _ts: i64) {
        self.0 = self.0.max(v);
    }
    fn merge(&mut self, other: &Self) {
        self.0 = self.0.max(other.0);
    }
    fn value(&self) -> f64 {
        self.0
    }
}

/// ASAPQuery's `CounterResetEvent`.
#[derive(Clone)]
struct ResetEvent {
    timestamp: i64,
    adjustment: f64,
}

/// ASAPQuery's `IncreaseAccumulator`, field for field.
#[derive(Clone)]
pub struct Increase {
    starting_value: f64,
    starting_timestamp: i64,
    last_seen_value: f64,
    last_seen_timestamp: i64,
    sample_count: u64,
    counter_reset_adjustment: f64,
    counter_reset_events: Vec<ResetEvent>,
}

impl Increase {
    /// Deduplicated by timestamp, as ASAPQuery's `add_reset_event`.
    fn add_reset_event(&mut self, event: ResetEvent) {
        if self
            .counter_reset_events
            .iter()
            .any(|e| e.timestamp == event.timestamp)
        {
            return;
        }
        self.counter_reset_adjustment += event.adjustment;
        self.counter_reset_events.push(event);
    }
}

impl Cell for Increase {
    fn first(v: f64, ts: i64) -> Self {
        Increase {
            starting_value: v,
            starting_timestamp: ts,
            last_seen_value: v,
            last_seen_timestamp: ts,
            sample_count: 1,
            counter_reset_adjustment: 0.0,
            counter_reset_events: Vec::new(),
        }
    }

    fn insert(&mut self, v: f64, ts: i64) {
        if v < self.last_seen_value {
            let adjustment = self.last_seen_value;
            self.counter_reset_adjustment += adjustment;
            self.counter_reset_events.push(ResetEvent {
                timestamp: ts,
                adjustment,
            });
        }
        self.last_seen_value = v;
        self.last_seen_timestamp = ts;
        self.sample_count += 1;
    }

    /// ASAPQuery's `merge_accumulators` over two: the earlier-starting side is
    /// the base, a drop at the seam is a reset, the later last sample wins.
    fn merge(&mut self, other: &Self) {
        let mut later = other.clone();
        if later.starting_timestamp < self.starting_timestamp {
            std::mem::swap(self, &mut later);
        }
        self.sample_count += later.sample_count;
        if later.starting_timestamp >= self.last_seen_timestamp
            && later.starting_value < self.last_seen_value
        {
            self.add_reset_event(ResetEvent {
                timestamp: later.starting_timestamp,
                adjustment: self.last_seen_value,
            });
        }
        for event in later.counter_reset_events {
            self.add_reset_event(event);
        }
        if later.last_seen_timestamp > self.last_seen_timestamp {
            self.last_seen_value = later.last_seen_value;
            self.last_seen_timestamp = later.last_seen_timestamp;
        }
    }

    /// `Statistic::Increase`, unbounded. Rate divides this by the timestamp
    /// span, which is a constant extra here.
    fn value(&self) -> f64 {
        self.last_seen_value - self.starting_value + self.counter_reset_adjustment
    }

    fn heap_bytes(&self) -> usize {
        self.counter_reset_events.capacity() * std::mem::size_of::<ResetEvent>()
    }
}

/// One accumulator holding every group.
pub struct Multiple<C> {
    by_group: HashMap<Vec<String>, C>,
}

impl<C: Cell> Multiple<C> {
    fn insert(&mut self, group: &[String], v: f64, ts: i64) {
        match self.by_group.get_mut(group) {
            Some(cell) => cell.insert(v, ts),
            None => {
                self.by_group.insert(group.to_vec(), C::first(v, ts));
            }
        }
    }

    fn merge(&mut self, other: &Self) {
        for (group, cell) in &other.by_group {
            match self.by_group.get_mut(group) {
                Some(mine) => mine.merge(cell),
                None => {
                    self.by_group.insert(group.clone(), cell.clone());
                }
            }
        }
    }

    fn answer(&self, group: &[String]) -> Result<f64, AggregateQueryError> {
        self.by_group
            .get(group)
            .map(Cell::value)
            .ok_or_else(|| AggregateQueryError::GroupNotFound(group.to_vec()))
    }

    /// Key and cell bytes per group; hash-table overhead left out.
    fn footprint(&self) -> usize {
        self.by_group
            .iter()
            .map(|(key, cell)| {
                std::mem::size_of::<Vec<String>>()
                    + key
                        .iter()
                        .map(|l| std::mem::size_of::<String>() + l.len())
                        .sum::<usize>()
                    + std::mem::size_of::<C>()
                    + cell.heap_bytes()
            })
            .sum()
    }
}

fn build<C: Cell>(params: &ParamSet) -> Result<Multiple<C>, BuildError> {
    let _: ExactParams = params.parse()?;
    Ok(Multiple {
        by_group: HashMap::new(),
    })
}

type Record<V> = (Vec<String>, V);
type Answer = Result<f64, AggregateQueryError>;

pub fn insert_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut acc = build::<C>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for (ts, (group, v)) in items.iter().enumerate() {
                acc.insert(group, v.to_f64(), ts as i64);
            }
            acc.footprint()
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let acc: Shared<_> = Rc::new(RefCell::new(build::<C>(params)?));
        let (driven, read) = (acc.clone(), acc);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let (group, v) = &stream[i];
                driven.borrow_mut().insert(group, v.to_f64(), i as i64);
            }),
            footprint: Box::new(move || read.borrow().footprint()),
        });
    }
    Ok(out)
}

pub fn query_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<Answer>>, BuildError> {
    merge_query_exact::<C, V>(params, items, probes, 1, passes)
}

/// The query, asked of the accumulator a fold over `shards` shards leaves.
pub fn merge_query_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    probes: Rc<Vec<Vec<String>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<Answer>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = exact_shards::<C, V>(params, &items, shards)?;
        for other in rest.iter() {
            acc.merge(other);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let answers = probes.iter().map(|p| acc.answer(p)).collect();
            (answers, acc.footprint())
        }) as QueryPass<Answer>);
    }
    Ok(out)
}

pub fn merge_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = exact_shards::<C, V>(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.merge(other);
            }
            acc.footprint()
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_exact<C: Cell, V: ToF64 + Copy + 'static>(
    params: &ParamSet,
    items: Rc<Vec<Record<V>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = exact_shards::<C, V>(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| driven.borrow_mut().merge(&rest[i])),
            footprint: Box::new(move || read.borrow().footprint()),
        });
    }
    Ok(out)
}

/// One accumulator per contiguous shard, timestamps continuing across them.
#[allow(clippy::type_complexity)]
fn exact_shards<C: Cell, V: ToF64 + Copy>(
    params: &ParamSet,
    items: &[Record<V>],
    shards: usize,
) -> Result<(Multiple<C>, Vec<Multiple<C>>), BuildError> {
    let mut parts = Vec::new();
    let mut ts = 0i64;
    for shard in crate::wrappers::partition(items, shards) {
        let mut acc = build::<C>(params)?;
        for (group, v) in shard {
            acc.insert(group, v.to_f64(), ts);
            ts += 1;
        }
        parts.push(acc);
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

    fn group(g: &str) -> Vec<String> {
        vec![g.to_string()]
    }

    /// Every split of the stream, resets inside a shard and across a seam,
    /// gives the single pass's per-group answer.
    #[test]
    fn increase_merged_in_any_split_matches_one_pass() {
        let items: Vec<Record<i64>> = [("a", 1), ("b", 10), ("a", 4), ("a", 2), ("b", 7), ("a", 9)]
            .into_iter()
            .map(|(g, v)| (group(g), v))
            .collect();
        let items = Rc::new(items);
        // a: 9 - 1 + 4 (reset at 4 -> 2) = 12; b: 7 - 10 + 10 (reset) = 7.
        let probes = Rc::new(vec![group("a"), group("b")]);
        let params = ParamSet::empty("exact-increase");
        for shards in 1..=items.len() {
            let mut q = merge_query_exact::<Increase, i64>(
                &params,
                items.clone(),
                probes.clone(),
                shards,
                1,
            )
            .unwrap();
            assert_eq!(q.remove(0)().0, vec![Ok(12.0), Ok(7.0)], "{shards} shards");
        }
    }

    #[test]
    fn a_group_never_seen_is_a_typed_error() {
        let items = Rc::new(vec![(group("a"), 1i64)]);
        let mut q = query_exact::<Sum, i64>(
            &ParamSet::empty("exact-sum"),
            items,
            Rc::new(vec![group("zz")]),
            1,
        )
        .unwrap();
        assert_eq!(
            q.remove(0)().0,
            vec![Err(AggregateQueryError::GroupNotFound(group("zz")))]
        );
    }
}
