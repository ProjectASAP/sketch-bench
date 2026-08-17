//! Turning "how one sketch is driven" into the closures a caller times.
//!
//! The functions themselves are written in the wrapper file that owns the
//! sketch, in that sketch's own terms — the `datasketches` Count-Min takes an
//! owned `i64`, the `asap_sketchlib` one takes a `&DataInput`, and each says so
//! in its own `insert`. Nothing here forces two of them to agree on anything.
//!
//! What lives here is the part that is the same for every sketch: given those
//! functions and a dataset, produce one [`Body`] per square the request
//! selected, each with its setup already done and only its own operation inside
//! the clock. This crate still names no sketch — `S`, `I`, `P` and `A` are
//! whatever the caller instantiated them at.

use std::rc::Rc;

use crate::accuracy::cardinality::CardinalityGT;
use crate::accuracy::frequency::FrequencyGT;
use crate::accuracy::quantile::RankErrorGT;
use crate::accuracy::subpopulation::{SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT};
use crate::accuracy::GroundTruth;
use crate::build_error::BuildError;
use crate::config::ParamSet;
use crate::dataset::spec::{BenchItem, DatasetSpec};
use crate::dataset::{Dataset, DatasetDescription, Labeled, LabeledDataset};
use crate::measure::{RunOutcome, Timed};
use crate::metrics::{cells, Metric, Operation};
use crate::request::Requirement;
use crate::run_error::RunError;
use aqpbm_datagen::ColumnItem;

// ---------- what a measurement does *to* a sketch ----------
//
// Every closure a measurement calls on a sketch is a parameter of
// [`squares_for`], not a field of some bundle. Each arrives as its own generic,
// so each is a zero-sized item the compiler inlines — which is what keeps the
// feed loop as fast as a hand-written one.
//
// `insert` is why it is done this way. Routing it through a pointer field cost
// the fixed-matrix row 17% (340M → 280M items/sec) when it was measured.
// Everything else runs once per measurement, once per probe or once per run, so
// a pointer would be free there — but there is no reason to spend one.
//
// `merge` and `prepare` are the exceptions, and are `Option<fn>`: their absence
// is a real declaration ("this library has no merge"), and `None` on a bare `fn`
// needs no turbofish at the call site.
//
// - `build(&ParamSet, workers) -> Result<S, BuildError>` — a fresh sketch. Called
//   per run, outside the timed region: every repeat of an insert has to start
//   from empty or it is not measuring the same thing twice. Takes the worker
//   count because the parallel rows need it and it is a *run* knob; every other
//   row ignores it.
// - `memory(&S) -> usize` — the footprint the sketch claims, read before the body
//   drops it.
// - `insert(&mut S, &I)` — one item in. The hot one.
// - `ask(&mut S, &P) -> A` — one question. `&mut` because several libraries need
//   it: `sketch_oxide`'s KLL sorts lazily on the first query.
// - `merge(&mut S, &S)` — absorb another built from the same `ParamSet`.
// - `prepare(&mut S)` — no more items are coming. Timed on its own, so a library
//   that defers its build is not credited with a fast insert loop.

// ---------- what a row hands the frontend ----------
//
// One closure per square, and nothing else. A body owns everything it needs —
// its own sketch, an `Rc` of the materialised items, its probes — so no two
// bodies share state and none borrows from this crate's stack. That is why a
// fresh sketch per run is free to be the rule: an insert measured ten times
// starts from empty ten times, and a query measured ten times pays its own fill
// each time, which matters because `sketch_oxide`'s KLL sorts lazily and a
// re-queried sketch is already sorted.
//
// Nothing here measures. The body marks the region it wants timed and says what
// it did; `aqpbm_core::measure` runs it and holds the clock. Which square a body
// is, and how many times to run it, are the frontend's to know.

/// One measurement, ready to be timed. Boxed because a body captures the sketch
/// it built, so every row's is a different anonymous type and a frontend needs
/// one type to hold them all. The virtual call lands once per run, outside the
/// region the body times.
pub type Body = Box<dyn FnMut(&mut Timed) -> RunOutcome>;

/// The smallest fold that is a merge at all: two shards, one merge call. The
/// count itself comes from the request, and this only keeps a `--merge-shards 1`
/// from measuring an empty loop.
pub const MIN_MERGE_SHARDS: usize = 2;

/// What the query squares ask, drawn once from the exact answer and shared by
/// however many of them the request selected.
type Asked<G, I> = (
    Rc<<G as GroundTruth<I>>::Truth>,
    Rc<Vec<<G as GroundTruth<I>>::Probe>>,
);

/// A body for every square the request selected, over one scored row, in
/// [`cells`] order — the caller computed the same masks, so it can label them by
/// calling the same function rather than being told.
///
/// Returned alongside them is the dataset's description, which is the one thing
/// about this run a frontend cannot work out for itself: `--input data.bin` has
/// no size or shape until the row has decoded it.
///
/// The dataset arrives behind an `Rc` because the squares share one
/// materialisation: four bodies over a million items is four handles, never
/// four copies.
///
/// Construction is proved once, here, before any body is handed back — a cell
/// that cannot be built fails whole, and never as a measurement of zero.
#[allow(clippy::too_many_arguments)]
pub fn squares_for<W, S, I, G, B, M, Ins, Ask>(
    req: &Requirement,
    dataset: Rc<W>,
    gt: G,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    W: Dataset<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    G: GroundTruth<I> + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
    Ask: Fn(&mut S, &G::Probe) -> G::Answer + Copy + 'static,
{
    let params = req.params.clone();
    // A run of no threads is not a run. Floored here rather than trusted from
    // the request, because a library caller builds its own `Requirement`.
    let workers = req.workers.max(1);
    build(&params, workers).map_err(|e| RunError::Body(e.to_string()))?;

    // Shared by every body: the comparator, and — for the query squares — the
    // exact answer and the questions drawn from it, computed once rather than
    // once per square.
    let gt = Rc::new(gt);
    let mut asking: Option<Asked<G, I>> = None;

    let mut bodies = Vec::new();
    for cell in cells(req.operations, req.metrics) {
        let body: Body = match cell.operation {
            Operation::Insert => {
                let (wk, params, metric) = (dataset.clone(), params.clone(), cell.metric);
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = build(&params, workers).expect("proven above");
                    match metric {
                        Metric::Latency => t.time_each(items, |v| insert(&mut s, v)),
                        _ => t.time(|| {
                            for v in items {
                                insert(&mut s, v);
                            }
                        }),
                    }
                    std::hint::black_box(&s);
                    RunOutcome {
                        work: items.len() as u64,
                        memory_bytes: Some(memory(&s) as u64),
                        ..Default::default()
                    }
                })
            }

            Operation::Query => {
                let (truth, asked) = asking
                    .get_or_insert_with(|| {
                        let truth = gt.truth(dataset.items());
                        let asked = gt.probes(&truth);
                        (Rc::new(truth), Rc::new(asked))
                    })
                    .clone();
                let (wk, params, gt, scored) = (
                    dataset.clone(),
                    params.clone(),
                    gt.clone(),
                    cell.metric == Metric::Accuracy,
                );
                Box::new(move |t: &mut Timed| {
                    // Setup: a fresh sketch, filled. Outside the clock, and
                    // redone every run so no run inherits another's warmed
                    // state.
                    let mut s = build(&params, workers).expect("proven above");
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
                })
            }

            Operation::Prepare => {
                let (wk, params) = (dataset.clone(), params.clone());
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = build(&params, workers).expect("proven above");
                    for v in items {
                        insert(&mut s, v);
                    }
                    t.time(|| {
                        if let Some(prepare) = prepare {
                            prepare(&mut s);
                        }
                    });
                    RunOutcome {
                        work: items.len() as u64,
                        memory_bytes: Some(memory(&s) as u64),
                        ..Default::default()
                    }
                })
            }

            Operation::Merge => {
                let Some(merge) = merge else {
                    return Err(RunError::Body(format!(
                        "{}/{} provides no merge",
                        req.algorithm, req.impl_name
                    )));
                };
                let shards = req.merge_shards.max(MIN_MERGE_SHARDS);
                let (wk, params) = (dataset.clone(), params.clone());
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let per = items.len().div_ceil(shards).max(1);
                    // Filling the shards is setup; only the fold is the
                    // measurement.
                    let mut parts: Vec<S> = items
                        .chunks(per)
                        .map(|chunk| {
                            let mut s = build(&params, workers).expect("proven above");
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
                })
            }
        };
        bodies.push(body);
    }

    Ok((dataset.description(), bodies))
}

/// The same, for a row nothing scores: insert, prepare and merge only, no
/// comparator and no probes.
#[allow(clippy::too_many_arguments)]
pub fn squares_for_unscored<W, S, I, B, M, Ins>(
    req: &Requirement,
    dataset: Rc<W>,
    build: B,
    memory: M,
    insert: Ins,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    W: Dataset<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    squares_for::<W, S, I, NoScore, B, M, Ins, _>(
        req,
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

/// The comparator for a row nothing scores. Computes no truth and asks nothing.
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

// ---------- the statistic a row answers ----------
//
// One entry point per statistic, and a wrapper calls the one its sketch answers.
//
// This is the seam that keeps a comparator out of `sketch-bench`. A wrapper
// states a fact about its *sketch* — "this grid answers subpopulation
// frequency" — and never a fact about the comparator. Which `GroundTruth`
// scores that statistic, and with which knobs, is decided below, in the crate
// that owns every comparator it could be. When a statistic grows a second
// comparator, only this section changes.
//
// The `ask` a wrapper passes is still typed in the probe and answer the
// statistic is asked in — `(String, V) -> f64` for subpopulation frequency —
// because `ask` has a signature and that signature is what makes the sketch an
// answer to *this* question rather than another. But those are structural types,
// not a comparator: a wrapper writes `(String, i64)`, not `SubpopFrequencyGT`.
//
// `label_column` is a comparator knob, not a sketch parameter, so it is chosen
// here. Column 0 is the coarsest grouping: the one whose cells the most records
// reach, and so the one a grid's shape is picked to control.
//
// These take a `&DatasetSpec` and materialise it themselves. The item type is
// already pinned by the `insert` they are handed — `Labeled<V>` names its own
// `Dataset` through `BenchItem` — so a wrapper never has to generate anything,
// and never has to know that a dataset has a spec, a size or a distribution.

/// The label column every subpopulation comparator scores over.
const SCORED_LABEL_COLUMN: usize = 0;

/// A row answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments)]
pub fn squares_subpop_frequency<S, V, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    V: ColumnItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &(String, V)) -> f64 + Copy + 'static,
{
    squares_for::<LabeledDataset<V>, S, Labeled<V>, SubpopFrequencyGT, B, M, Ins, Ask>(
        req,
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

/// A row answering **subpopulation cardinality**: how many distinct values a
/// group holds. The statistic a Count-Min cell structurally cannot reach.
#[allow(clippy::too_many_arguments)]
pub fn squares_subpop_cardinality<S, V, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    V: ColumnItem + Eq + std::hash::Hash + Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &String) -> f64 + Copy + 'static,
{
    squares_for::<LabeledDataset<V>, S, Labeled<V>, SubpopCardinalityGT, B, M, Ins, Ask>(
        req,
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

/// A row answering **subpopulation quantile**: the ordered statistic inside a
/// group, scored in rank error.
#[allow(clippy::too_many_arguments)]
pub fn squares_subpop_quantile<S, V, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    V: ColumnItem + crate::accuracy::quantile::QuantileValue + Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &Labeled<V>) + Copy + 'static,
    Ask: Fn(&mut S, &(String, f64)) -> f64 + Copy + 'static,
{
    squares_for::<LabeledDataset<V>, S, Labeled<V>, SubpopRankErrorGT, B, M, Ins, Ask>(
        req,
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

/// A row answering **frequency**: how often a key occurs in the stream.
#[allow(clippy::too_many_arguments)]
pub fn squares_frequency<S, K, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + Ord + Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &K) -> u64 + Copy + 'static,
{
    squares_for::<K::Wk, S, K, FrequencyGT, B, M, Ins, Ask>(
        req,
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

/// A row answering **cardinality**: how many distinct keys the stream carried.
///
/// The probe is `()` — there is one question, asked repeatedly — so `ask` ignores
/// it and returns the estimate.
#[allow(clippy::too_many_arguments)]
pub fn squares_cardinality<S, K, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    K: BenchItem + Eq + std::hash::Hash + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &K) + Copy + 'static,
    Ask: Fn(&mut S, &()) -> f64 + Copy + 'static,
{
    squares_for::<K::Wk, S, K, CardinalityGT, B, M, Ins, Ask>(
        req,
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

/// A row answering **quantile**, scored in rank error: the value at a fraction of
/// the sorted stream.
#[allow(clippy::too_many_arguments)]
pub fn squares_quantile<S, I, B, M, Ins, Ask>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    ask: Ask,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    I: BenchItem + Clone + PartialOrd + crate::accuracy::quantile::ToF64 + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
    Ask: Fn(&mut S, &f64) -> f64 + Copy + 'static,
{
    squares_for::<I::Wk, S, I, RankErrorGT, B, M, Ins, Ask>(
        req,
        Rc::new(spec.build::<I>()?),
        RankErrorGT {},
        build,
        memory,
        insert,
        ask,
        merge,
        prepare,
    )
}

/// A row that answers **nothing**: measured but not scored. The parallel-insert
/// rows, whose worker sketches are dropped rather than asked.
#[allow(clippy::too_many_arguments)]
pub fn squares_timed_only<S, I, B, M, Ins>(
    req: &Requirement,
    spec: &DatasetSpec,
    build: B,
    memory: M,
    insert: Ins,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<(DatasetDescription, Vec<Body>), RunError>
where
    S: 'static,
    I: BenchItem + Clone + 'static,
    B: Fn(&ParamSet, usize) -> Result<S, BuildError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    squares_for_unscored::<I::Wk, S, I, B, M, Ins>(
        req,
        Rc::new(spec.build::<I>()?),
        build,
        memory,
        insert,
        merge,
        prepare,
    )
}
