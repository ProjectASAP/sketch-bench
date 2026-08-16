//! How one sketch is driven, as functions — and the closures a row hands back.
//!
//! Six of them, written in the wrapper file that owns the sketch, in that
//! sketch's own terms. Nothing here forces two rows to agree on anything: the
//! `datasketches` Count-Min takes an owned `i64`, the `asap_sketchlib` one
//! takes a `&DataInput`, and each says so in its own `insert`.

use std::rc::Rc;

use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::RunError;
use aqpbm_core::config::ParamSet;
use aqpbm_core::measure::{RunOutcome, Timed};
use aqpbm_core::metrics::{cells, Metric, Operation};
use aqpbm_core::request::Requirement;
use aqpbm_core::workload::{Workload, WorkloadDescription};

use crate::build_error::BuildError;

/// Everything a measurement does *to* a sketch.
///
/// - `S` the sketch, `I` what it ingests.
/// - `P` one question, `A` one answer — both fixed by the row's comparator,
///   because a comparison only means anything if both sides answer the same
///   question.
///
/// `insert` is deliberately **not** here: it travels as a generic so it stays a
/// zero-sized `fn` item and inlines. Routing it through a pointer field cost
/// the fixed-matrix row 17% (340M → 280M items/sec) when it was measured.
/// Everything below runs once per measurement, or once per probe, so a pointer
/// is free there.
pub struct SketchOps<S, I, P, A> {
    /// Build a fresh one. Called per run, outside the timed region: every
    /// repeat of an insert has to start from empty or it is not measuring the
    /// same thing twice.
    ///
    /// Takes the worker count as well as the params, because the parallel rows
    /// need it and it is a *run* knob rather than a sketch parameter. Every
    /// other row ignores it.
    pub build: fn(&ParamSet, usize) -> Result<S, BuildError>,
    /// The footprint the sketch claims, read before the body drops it.
    pub memory: fn(&S) -> usize,
    /// Put one question to it. `&mut` because several libraries need it —
    /// `sketch_oxide`'s KLL sorts lazily on the first query.
    pub ask: fn(&mut S, &P) -> A,
    /// Absorb another built from the same `ParamSet`. `None` when the library
    /// has no merge — the honest answer, not a zero.
    pub merge: Option<fn(&mut S, &S)>,
    /// No more items are coming. Timed on its own, so a library that defers
    /// its build is not credited with a fast insert loop.
    pub prepare: Option<fn(&mut S)>,
    /// `I` appears only in `insert`, which lives outside this struct.
    pub _item: std::marker::PhantomData<fn(&I)>,
}

impl<S, I, P, A> SketchOps<S, I, P, A> {
    /// Run `prepare` if this row has one.
    #[inline(always)]
    pub fn run_prepare(&self, sketch: &mut S) {
        if let Some(prepare) = self.prepare {
            prepare(sketch);
        }
    }
}

impl<S, I, P, A> Clone for SketchOps<S, I, P, A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S, I, P, A> Copy for SketchOps<S, I, P, A> {}

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
/// Returned alongside them is the workload's description, which is the one thing
/// about this run a frontend cannot work out for itself: `--input data.bin` has
/// no size or shape until the row has decoded it.
///
/// The workload arrives behind an `Rc` because the squares share one
/// materialisation: four bodies over a million items is four handles, never
/// four copies.
///
/// Construction is proved once, here, before any body is handed back — a cell
/// that cannot be built fails whole, and never as a measurement of zero.
pub fn squares_for<W, S, I, G, Ins>(
    req: &Requirement,
    workload: Rc<W>,
    gt: G,
    insert: Ins,
    ops: SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<(WorkloadDescription, Vec<Body>), RunError>
where
    W: Workload<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    G: GroundTruth<I> + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    let params = req.params.clone();
    // A run of no threads is not a run. Floored here rather than trusted from
    // the request, because a library caller builds its own `Requirement`.
    let workers = req.workers.max(1);
    (ops.build)(&params, workers).map_err(|e| RunError::Body(e.to_string()))?;

    // Shared by every body: the comparator, and — for the query squares — the
    // exact answer and the questions drawn from it, computed once rather than
    // once per square.
    let gt = Rc::new(gt);
    let mut asking: Option<Asked<G, I>> = None;

    let mut bodies = Vec::new();
    for cell in cells(req.operations, req.metrics) {
        let body: Body = match cell.operation {
            Operation::Insert => {
                let (wk, params, metric) = (workload.clone(), params.clone(), cell.metric);
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = (ops.build)(&params, workers).expect("proven above");
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
                        memory_bytes: Some((ops.memory)(&s) as u64),
                        ..Default::default()
                    }
                })
            }

            Operation::Query => {
                let (truth, asked) = asking
                    .get_or_insert_with(|| {
                        let truth = gt.truth(workload.items());
                        let asked = gt.probes(&truth);
                        (Rc::new(truth), Rc::new(asked))
                    })
                    .clone();
                let (wk, params, gt, scored) = (
                    workload.clone(),
                    params.clone(),
                    gt.clone(),
                    cell.metric == Metric::Accuracy,
                );
                Box::new(move |t: &mut Timed| {
                    // Setup: a fresh sketch, filled. Outside the clock, and
                    // redone every run so no run inherits another's warmed
                    // state.
                    let mut s = (ops.build)(&params, workers).expect("proven above");
                    for v in wk.items() {
                        insert(&mut s, v);
                    }
                    ops.run_prepare(&mut s);

                    let mut answers = Vec::with_capacity(asked.len());
                    t.time(|| {
                        for p in asked.iter() {
                            answers.push((ops.ask)(&mut s, p));
                        }
                    });
                    RunOutcome {
                        work: asked.len() as u64,
                        memory_bytes: Some((ops.memory)(&s) as u64),
                        scores: if scored {
                            gt.score(&truth, &asked, &answers)
                        } else {
                            Default::default()
                        },
                    }
                })
            }

            Operation::Prepare => {
                let (wk, params) = (workload.clone(), params.clone());
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let mut s = (ops.build)(&params, workers).expect("proven above");
                    for v in items {
                        insert(&mut s, v);
                    }
                    t.time(|| ops.run_prepare(&mut s));
                    RunOutcome {
                        work: items.len() as u64,
                        memory_bytes: Some((ops.memory)(&s) as u64),
                        ..Default::default()
                    }
                })
            }

            Operation::Merge => {
                let Some(merge) = ops.merge else {
                    return Err(RunError::Body(format!(
                        "{}/{} provides no merge",
                        req.algorithm, req.impl_name
                    )));
                };
                let shards = req.merge_shards.max(MIN_MERGE_SHARDS);
                let (wk, params) = (workload.clone(), params.clone());
                Box::new(move |t: &mut Timed| {
                    let items = wk.items();
                    let per = items.len().div_ceil(shards).max(1);
                    // Filling the shards is setup; only the fold is the
                    // measurement.
                    let mut parts: Vec<S> = items
                        .chunks(per)
                        .map(|chunk| {
                            let mut s = (ops.build)(&params, workers).expect("proven above");
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
                        memory_bytes: Some((ops.memory)(&acc) as u64),
                        ..Default::default()
                    }
                })
            }
        };
        bodies.push(body);
    }

    Ok((workload.description(), bodies))
}

/// The same, for a row nothing scores: insert, prepare and merge only, no
/// comparator and no probes.
pub fn squares_for_unscored<W, S, I, Ins>(
    req: &Requirement,
    workload: Rc<W>,
    insert: Ins,
    ops: SketchOps<S, I, (), ()>,
) -> Result<(WorkloadDescription, Vec<Body>), RunError>
where
    W: Workload<Item = I> + 'static,
    S: 'static,
    I: Clone + 'static,
    Ins: Fn(&mut S, &I) + Copy + 'static,
{
    squares_for::<W, S, I, NoScore, Ins>(req, workload, NoScore, insert, ops)
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
