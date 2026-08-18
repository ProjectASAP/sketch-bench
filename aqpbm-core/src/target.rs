//! Turning "how one sketch is driven" into the closures a caller times. The
//! per-sketch functions live in the wrapper that owns the sketch; what lives
//! here is what every sketch shares — one [`Measurement`] per instruction.
//!
//! A frontend opens a [`Target`] and then tells it what to do, one measurement at
//! a time. Core does not decide which measurements to take; see
//! `docs/aqpbm-core.md` §Input.

use std::marker::PhantomData;
use std::rc::Rc;

use crate::accuracy::cardinality::CardinalityGT;
use crate::accuracy::frequency::FrequencyGT;
use crate::accuracy::heavy_hitter::HeavyHitterGT;
use crate::accuracy::quantile::RankErrorGT;
use crate::accuracy::subpopulation::{SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT};
use crate::accuracy::topk::TopkGT;
use crate::accuracy::GroundTruth;
use crate::error::RunError;
use crate::input_dataset::spec::{BenchItem, InputDataSetSpec};
use crate::input_dataset::{InputDataSet, InputDataSetDescription, Labeled, LabeledInputDataSet};
use crate::measure::{RunOutcome, Timed};
use crate::metrics::{Metric, Operation};
use aqpbm_datagen::ColumnItem;

pub type Measurement = Box<dyn FnMut(&mut Timed) -> RunOutcome>;

/// What it takes to open a target, as one value. The construction parameters
/// are whatever the caller's vocabulary is: core hands `P` back to the build
/// closure and never reads it.
#[derive(Clone, Debug)]
pub struct Opening<P> {
    pub algorithm: String,
    pub impl_name: String,
    pub params: P,
    /// A run of no threads is not a run; floored on the way in.
    pub workers: usize,
    pub merge_shards: usize,
}

/// The smallest fold that is a merge at all: two shards, one merge call. The
/// count itself comes from the request, and this only keeps a `--merge-shards 1`
/// from measuring an empty loop.
pub const MIN_MERGE_SHARDS: usize = 2;

/// An opened target: its dataset materialised, its comparator ready, waiting to be
/// told what to do.
///
/// One is opened per invocation and asked for one measurement at a time. The
/// data and the exact answer are computed once here and shared by every
/// instruction that follows, which is what lets a frontend hand the same data
/// over as many times as it likes without regenerating it.
pub trait Target {
    /// What the materialised data was, for the record.
    fn description(&self) -> InputDataSetDescription;

    /// One instruction: perform `operation`, timed the way `metric` needs.
    ///
    /// Errors when the target provides no merge or no prepare — timing a `None`
    /// would report `work = items.len()` against a zero-length region, which
    /// reads as an infinitely fast operation rather than as a missing one.
    /// `registry::check` catches this for a CLI request, but a caller driving
    /// this directly does not go through it.
    fn body(&mut self, operation: Operation, metric: Metric) -> Result<Measurement, RunError>;
}

/// What the query measurements ask, drawn once from the exact answer and shared
/// by however many of them a frontend asks for.
type Asked<G, I> = (
    Rc<<G as GroundTruth<I>>::Truth>,
    Rc<Vec<<G as GroundTruth<I>>::Probe>>,
);

/// The one [`Target`] implementation: everything a body needs, held until asked.
/// The dataset rides an `Rc` because the bodies share one materialisation, and
/// construction is proved in [`open_target_scored`] before any body ships.
struct Opened<P, W, S, I, G, B, M, Ins, Ask>
where
    G: GroundTruth<I>,
{
    algorithm: String,
    impl_name: String,
    params: P,
    /// A run of no threads is not a run. Floored on the way in rather than
    /// trusted from the request, because a library caller builds its own.
    workers: usize,
    merge_shards: usize,
    dataset: Rc<W>,
    gt: Rc<G>,
    /// The exact answer and the questions drawn from it — computed on the first
    /// query instruction and reused by any that follow, rather than once per
    /// measurement.
    asked: Option<Asked<G, I>>,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
    _item: PhantomData<I>,
}

impl<P, W, S, I, G, B, M, Ins, Ask> Target for Opened<P, W, S, I, G, B, M, Ins, Ask>
where
    P: Clone + 'static,
    W: InputDataSet<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    G: GroundTruth<I> + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
    Ask: Fn(&mut S, &G::Probe) -> G::Answer + Copy + 'static,
{
    fn description(&self) -> InputDataSetDescription {
        self.dataset.description()
    }

    fn body(&mut self, operation: Operation, metric: Metric) -> Result<Measurement, RunError> {
        let workers = self.workers;
        let (build, memory, insert, ask) = (self.build, self.memory, self.insert, self.ask);

        match (operation, metric) {
            (Operation::Insert, Metric::Throughput) | (Operation::Insert, Metric::Latency) => {
                let per_call = metric == Metric::Latency;
                let (wk, params) = (self.dataset.clone(), self.params.clone());
                Ok(Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = build(&params, workers).expect("proven at open");
                    if per_call {
                        t.time_each(items, |v| insert(&mut s, v));
                    } else {
                        t.time(|| {
                            for v in items {
                                insert(&mut s, v);
                            }
                        });
                    }
                    std::hint::black_box(&s);
                    RunOutcome {
                        work: items.len() as u64,
                        memory_bytes: Some(memory(&s) as u64),
                        ..Default::default()
                    }
                }))
            }

            (Operation::Query, Metric::Throughput) | (Operation::Query, Metric::Accuracy) => {
                // Disjoint field borrows: the memo is filled from the
                // comparator and the data without borrowing all of `self`.
                let (gt, dataset) = (&self.gt, &self.dataset);
                let (truth, asked) = self
                    .asked
                    .get_or_insert_with(|| {
                        let truth = gt.truth(dataset.items());
                        let asked = gt.probes(&truth);
                        (Rc::new(truth), Rc::new(asked))
                    })
                    .clone();
                let (wk, params, gt, prepare) = (
                    self.dataset.clone(),
                    self.params.clone(),
                    self.gt.clone(),
                    self.prepare,
                );
                let scored = metric == Metric::Accuracy;
                Ok(Box::new(move |t: &mut Timed| {
                    // Setup: a fresh sketch, filled. Outside the clock, and
                    // redone every run so no run inherits another's warmed
                    // state.
                    let mut s = build(&params, workers).expect("proven at open");
                    for v in wk.items() {
                        insert(&mut s, v);
                    }
                    if let Some(prepare) = prepare {
                        prepare(&mut s);
                    }

                    let mut answers = Vec::with_capacity(asked.len());
                    t.time(|| {
                        for p in asked.iter() {
                            answers.push(ask(&mut s, p));
                        }
                    });
                    RunOutcome {
                        work: asked.len() as u64,
                        memory_bytes: Some(memory(&s) as u64),
                        scores: if scored {
                            gt.score(&truth, &asked, &answers)
                        } else {
                            Default::default()
                        },
                    }
                }))
            }

            (Operation::Prepare, Metric::Latency) => {
                let Some(prepare) = self.prepare else {
                    return Err(RunError::Target(format!(
                        "{}/{} provides no prepare",
                        self.algorithm, self.impl_name
                    )));
                };
                let (wk, params) = (self.dataset.clone(), self.params.clone());
                Ok(Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = build(&params, workers).expect("proven at open");
                    for v in items {
                        insert(&mut s, v);
                    }
                    t.time(|| prepare(&mut s));
                    RunOutcome {
                        work: items.len() as u64,
                        memory_bytes: Some(memory(&s) as u64),
                        ..Default::default()
                    }
                }))
            }

            (Operation::Merge, Metric::Throughput) | (Operation::Merge, Metric::Latency) => {
                let Some(merge) = self.merge else {
                    return Err(RunError::Target(format!(
                        "{}/{} provides no merge",
                        self.algorithm, self.impl_name
                    )));
                };
                let shards = self.merge_shards.max(MIN_MERGE_SHARDS);
                let (wk, params) = (self.dataset.clone(), self.params.clone());
                Ok(Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let per = items.len().div_ceil(shards).max(1);
                    // Filling the shards is setup; only the fold is the
                    // measurement.
                    let mut parts: Vec<S> = items
                        .chunks(per)
                        .map(|chunk| {
                            let mut s = build(&params, workers).expect("proven at open");
                            for v in chunk {
                                insert(&mut s, v);
                            }
                            s
                        })
                        .collect();
                    let rest = parts.split_off(1);
                    let mut acc = parts.pop().expect("chunks yields at least one");
                    t.time(|| {
                        for other in &rest {
                            merge(&mut acc, other);
                        }
                    });
                    std::hint::black_box(&acc);
                    RunOutcome {
                        work: rest.len() as u64,
                        memory_bytes: Some(memory(&acc) as u64),
                        ..Default::default()
                    }
                }))
            }

            (operation, metric) => Err(RunError::Target(format!(
                "{}/{} cannot measure {} over {}",
                self.algorithm,
                self.impl_name,
                metric.name(),
                operation.name()
            ))),
        }
    }
}

/// Open a target over an already-materialised dataset. Construction is proved here,
/// once, so a config the target cannot satisfy fails before any measurement
/// is asked for.
#[allow(clippy::too_many_arguments)]
pub fn open_target_scored<P, W, S, I, G, B, M, Ins, Ask>(
    opening: &Opening<P>,
    dataset: Rc<W>,
    gt: G,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    W: InputDataSet<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    G: GroundTruth<I> + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
    Ask: Fn(&mut S, &G::Probe) -> G::Answer + Copy + 'static,
{
    let workers = opening.workers.max(1);
    build(&opening.params, workers)?;

    Ok(Box::new(Opened {
        algorithm: opening.algorithm.clone(),
        impl_name: opening.impl_name.clone(),
        params: opening.params.clone(),
        workers,
        merge_shards: opening.merge_shards,
        dataset,
        gt: Rc::new(gt),
        asked: None,
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
        _item: PhantomData,
    }))
}

/// The same, for a target nothing scores: insert, prepare and merge only, no
/// comparator and no probes.
#[allow(clippy::too_many_arguments)]
pub fn open_target_unscored<P, W, S, I, B, M, Ins>(
    opening: &Opening<P>,
    dataset: Rc<W>,
    build: B,
    memory: M,
    insert: Ins,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    W: InputDataSet<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    open_target_scored::<P, W, S, I, NoScore, B, M, Ins, _>(
        opening,
        dataset,
        NoScore,
        build,
        memory,
        insert,
        |_: &mut S, _: &()| (),
        merge,
        prepare,
    )
}

/// The comparator for a target nothing scores. Computes no truth and asks nothing.
pub struct NoScore;

impl<I> GroundTruth<I> for NoScore {
    type Truth = ();
    type Probe = ();
    type Answer = ();
    fn truth(&self, _: &[I]) {}
    fn probes(&self, _: &()) -> Vec<()> {
        Vec::new()
    }
    fn score(&self, _: &(), _: &[()], _: &[()]) -> std::collections::BTreeMap<String, f64> {
        Default::default()
    }
}

// ---------- the statistic a target answers ----------

/// The label column every subpopulation comparator scores over.
const SCORED_LABEL_COLUMN: usize = 0;

/// A target answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments)]
pub fn open_target_subpop_frequency<P, S, V, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    V: ColumnItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &(String, V)) -> f64 + Copy + 'static,
{
    open_target_scored::<P, LabeledInputDataSet<V>, S, Labeled<V>, SubpopFrequencyGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<Labeled<V>>()?),
        SubpopFrequencyGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **subpopulation cardinality**: how many distinct values a
/// group holds. The statistic a Count-Min cell structurally cannot reach.
#[allow(clippy::too_many_arguments)]
pub fn open_target_subpop_cardinality<P, S, V, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    V: ColumnItem + Eq + std::hash::Hash + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &String) -> f64 + Copy + 'static,
{
    open_target_scored::<
        P,
        LabeledInputDataSet<V>,
        S,
        Labeled<V>,
        SubpopCardinalityGT,
        B,
        M,
        Ins,
        Ask,
    >(
        opening,
        Rc::new(spec.build::<Labeled<V>>()?),
        SubpopCardinalityGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **subpopulation quantile**: the ordered statistic inside a
/// group, scored in rank error.
#[allow(clippy::too_many_arguments)]
pub fn open_target_subpop_quantile<P, S, V, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    V: ColumnItem + crate::accuracy::quantile::QuantileValue + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &(String, f64)) -> f64 + Copy + 'static,
{
    open_target_scored::<P, LabeledInputDataSet<V>, S, Labeled<V>, SubpopRankErrorGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<Labeled<V>>()?),
        SubpopRankErrorGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **frequency**: how often a key occurs in the stream.
#[allow(clippy::too_many_arguments)]
pub fn open_target_frequency<P, S, K, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &K) -> u64 + Copy + 'static,
{
    open_target_scored::<P, K::Wk, S, K, FrequencyGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<K>()?),
        FrequencyGT,
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **top-k**: which `k` keys are heaviest, and how heavy.
/// `k` is the comparator's parameter, not the sketch's, so it arrives here
/// rather than on the request. The target's `ask` must use the same `k`.
#[allow(clippy::too_many_arguments)]
pub fn open_target_topk<P, S, K, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    k: usize,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &()) -> Vec<(K, u64)> + Copy + 'static,
{
    open_target_scored::<P, K::Wk, S, K, TopkGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<K>()?),
        TopkGT { k },
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **heavy-hitter**: the heavy items and how heavy.
/// `phi` is this comparator's bound — heavy means `count > phi * n` — and
/// travels with the statistic. The target's `ask` must draw the same line.
#[allow(clippy::too_many_arguments)]
pub fn open_target_heavy_hitter<P, S, K, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    phi: f64,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &()) -> Vec<(K, u64)> + Copy + 'static,
{
    open_target_scored::<P, K::Wk, S, K, HeavyHitterGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<K>()?),
        HeavyHitterGT { phi },
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **cardinality**: how many distinct keys the stream carried.
///
/// The probe is `()` — there is one question, asked repeatedly — so `ask` ignores
/// it and returns the estimate.
#[allow(clippy::too_many_arguments)]
pub fn open_target_cardinality<P, S, K, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &()) -> f64 + Copy + 'static,
{
    open_target_scored::<P, K::Wk, S, K, CardinalityGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<K>()?),
        CardinalityGT,
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target answering **quantile**, scored in rank error: the value at a fraction of
/// the sorted stream.
#[allow(clippy::too_many_arguments)]
pub fn open_target_quantile<P, S, I, B, M, Ins, Ask>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    I: BenchItem + Clone + PartialOrd + crate::accuracy::quantile::ToF64 + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
    Ask: Fn(&mut S, &f64) -> f64 + Copy + 'static,
{
    open_target_scored::<P, I::Wk, S, I, RankErrorGT, B, M, Ins, Ask>(
        opening,
        Rc::new(spec.build::<I>()?),
        RankErrorGT,
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A target that answers **nothing**: measured but not scored. The parallel-insert
/// rows, whose worker sketches are dropped rather than asked.
#[allow(clippy::too_many_arguments)]
pub fn open_target_timed_only<P, S, I, B, M, Ins>(
    opening: &Opening<P>,
    spec: &InputDataSetSpec,
    build: B,
    memory: M,
    insert: Ins,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Box<dyn Target>, RunError>
where
    P: Clone + 'static,
    S: 'static,
    I: BenchItem + Clone + 'static,
    B: Fn(&P, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    open_target_unscored::<P, I::Wk, S, I, B, M, Ins>(
        opening,
        Rc::new(spec.build::<I>()?),
        build,
        memory,
        insert,
        merge,
        prepare,
    )
}
