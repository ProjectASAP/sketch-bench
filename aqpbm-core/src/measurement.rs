//! Turning "how one sketch is driven" into the closures a caller times. The
//! per-sketch functions live in the wrapper that owns the sketch; what lives
//! here is what every sketch shares — one [`Measurement`] per instruction.
//!
//! One builder per [`Operation`](crate::metrics::Operation), the metric as an
//! argument. Core does not decide which measurements to take, and it no longer
//! decides where the items come from: both arrive already made. See
//! `docs/aqpbm-core.md` §Input.

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::accuracy::GroundTruth;
use crate::error::RunError;
use crate::measure::{RunOutcome, Timed};
use crate::metrics::Metric;

pub type Measurement = Box<dyn FnMut(&mut Timed) -> RunOutcome>;

/// The smallest fold that is a merge at all: two shards, one merge call. The
/// count itself comes from the request, and this only keeps a `--merge-shards 1`
/// from measuring an empty loop.
pub const MIN_MERGE_SHARDS: usize = 2;

/// How to make the thing being measured, and how to weigh one. Neither is an
/// operation: `build` is construction, and `footprint` is read once the clock
/// has stopped, because a body that owns its sketch drops it on the way out.
///
/// `build` takes nothing: whatever the caller's construction vocabulary is, it
/// is already captured, so core never sees a parameter it cannot read.
#[derive(Clone, Copy)]
pub struct Sketch<B, M> {
    pub build: B,
    pub footprint: M,
}

/// The two forms an insert takes. A row that ingests one item at a time can be
/// timed per item; a row whose ingest *is* the whole slice cannot, and says so
/// rather than timing a loop that only buffers.
pub enum Insert<Per, S, I> {
    PerItem(Per),
    Bulk(fn(&mut S, &[I])),
}

impl<Per: Clone, S, I> Clone for Insert<Per, S, I> {
    fn clone(&self) -> Self {
        match self {
            Insert::PerItem(insert) => Insert::PerItem(insert.clone()),
            Insert::Bulk(ingest) => Insert::Bulk(*ingest),
        }
    }
}

impl<Per: Copy, S, I> Copy for Insert<Per, S, I> {}

/// One item at a time, the form that can also be timed per call.
pub fn per_item<S, I, Per>(insert: Per) -> Insert<Per, S, I>
where
    Per: Fn(&mut S, &I) + Copy,
{
    Insert::PerItem(insert)
}

/// The whole slice at once. Static dispatch buys nothing here — the call
/// happens once per run, not once per item — so the pointer is concrete and
/// the unused per-item slot goes with it.
pub fn bulk<S, I>(insert: fn(&mut S, &[I])) -> Insert<fn(&mut S, &I), S, I> {
    Insert::Bulk(insert)
}

/// The form is read once, outside the loop, so the per-item arm compiles to the
/// loop it would have been written as.
#[inline(always)]
fn fill<S, I, Per>(insert: &Insert<Per, S, I>, sketch: &mut S, items: &[I])
where
    Per: Fn(&mut S, &I) + Copy,
{
    match insert {
        Insert::PerItem(insert) => {
            for v in items {
                insert(sketch, v);
            }
        }
        Insert::Bulk(ingest) => ingest(sketch, items),
    }
}

/// What the query measurements ask, and how their answers score. Drawn once by
/// the caller from the exact answer, so however many query measurements follow,
/// they share one set of questions and no comparator type reaches a body.
pub struct Questions<P, Sc> {
    pub probes: Rc<Vec<P>>,
    pub score: Sc,
}

impl<P, Sc: Clone> Clone for Questions<P, Sc> {
    fn clone(&self) -> Self {
        Questions {
            probes: self.probes.clone(),
            score: self.score.clone(),
        }
    }
}

/// Draw them: the exact answer over the whole stream, the questions off it, and
/// a scorer holding both. The one place core still reads a [`GroundTruth`], and
/// it runs once per row, outside every clock.
#[allow(clippy::type_complexity)]
pub fn questions<G, I>(
    gt: G,
    items: &[I],
) -> Questions<G::Probe, impl Fn(&[G::Answer]) -> BTreeMap<String, f64> + Clone + 'static>
where
    G: GroundTruth<I> + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
{
    let gt = Rc::new(gt);
    let truth = Rc::new(gt.truth(items));
    let probes = Rc::new(gt.probes(&truth));
    let score = {
        let (gt, truth, probes) = (gt.clone(), truth.clone(), probes.clone());
        move |answers: &[G::Answer]| gt.score(&truth, &probes, answers)
    };
    Questions { probes, score }
}

/// **Insert**: build one, feed it the stream, report what it cost.
pub fn insert_measurement<S, I, B, M, Per>(
    metric: Metric,
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
) -> Result<Measurement, RunError>
where
    S: 'static,
    I: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
{
    let Sketch { build, footprint } = sketch;
    match metric {
        Metric::Throughput => Ok(Box::new(move |t: &mut Timed| {
            let items = items.as_slice();
            let mut s = build().expect("proven before any measurement ships");
            t.time(|| fill(&insert, &mut s, items));
            std::hint::black_box(&s);
            RunOutcome {
                work: items.len() as u64,
                memory_bytes: Some(footprint(&s) as u64),
                ..Default::default()
            }
        })),
        Metric::Latency => {
            let Insert::PerItem(insert) = insert else {
                return Err(RunError::Sketch(
                    "ingests the stream in one call, so there is no per-item region to time"
                        .to_string(),
                ));
            };
            Ok(Box::new(move |t: &mut Timed| {
                let items = items.as_slice();
                let mut s = build().expect("proven before any measurement ships");
                t.time_each(items, |v| insert(&mut s, v));
                std::hint::black_box(&s);
                RunOutcome {
                    work: items.len() as u64,
                    memory_bytes: Some(footprint(&s) as u64),
                    ..Default::default()
                }
            }))
        }
        Metric::Accuracy => Err(RunError::Sketch(
            "an insert returns no answer, so there is nothing to score".to_string(),
        )),
    }
}

/// **Query**: fill one, then ask it every question. Filling is setup and is
/// redone every run, so no run inherits another's warmed state.
#[allow(clippy::too_many_arguments)]
pub fn query_measurement<S, I, P, A, B, M, Per, Q, Sc>(
    metric: Metric,
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
    prepare: Option<fn(&mut S)>,
    query: Q,
    questions: Questions<P, Sc>,
) -> Result<Measurement, RunError>
where
    S: 'static,
    I: 'static,
    P: 'static,
    A: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
    Q: Fn(&mut S, &P) -> A + Copy + 'static,
    Sc: Fn(&[A]) -> BTreeMap<String, f64> + Clone + 'static,
{
    let scored = match metric {
        Metric::Throughput => false,
        Metric::Accuracy => true,
        Metric::Latency => {
            return Err(RunError::Sketch(
                "a query is timed over the whole probe set, not per call".to_string(),
            ))
        }
    };
    let Sketch { build, footprint } = sketch;
    let Questions { probes, score } = questions;
    Ok(Box::new(move |t: &mut Timed| {
        // Setup: a fresh sketch, filled, prepared. Outside the clock.
        let mut s = build().expect("proven before any measurement ships");
        fill(&insert, &mut s, items.as_slice());
        if let Some(prepare) = prepare {
            prepare(&mut s);
        }

        let mut answers = Vec::with_capacity(probes.len());
        t.time(|| {
            for p in probes.iter() {
                answers.push(query(&mut s, p));
            }
        });
        RunOutcome {
            work: probes.len() as u64,
            memory_bytes: Some(footprint(&s) as u64),
            scores: if scored {
                score(&answers)
            } else {
                Default::default()
            },
        }
    }))
}

/// **Merge**: fill `shards` of them, then fold. Filling the shards is setup;
/// only the fold is the measurement.
pub fn merge_measurement<S, I, B, M, Per>(
    metric: Metric,
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
    merge: fn(&mut S, &S),
    shards: usize,
) -> Result<Measurement, RunError>
where
    S: 'static,
    I: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
{
    let per_call = match metric {
        Metric::Throughput => false,
        Metric::Latency => true,
        Metric::Accuracy => {
            return Err(RunError::Sketch(
                "a fold returns no answer, so there is nothing to score".to_string(),
            ))
        }
    };
    let Sketch { build, footprint } = sketch;
    let shards = shards.max(MIN_MERGE_SHARDS);
    Ok(Box::new(move |t: &mut Timed| {
        let items = items.as_slice();
        let per = items.len().div_ceil(shards).max(1);
        let mut parts: Vec<S> = items
            .chunks(per)
            .map(|chunk| {
                let mut s = build().expect("proven before any measurement ships");
                fill(&insert, &mut s, chunk);
                s
            })
            .collect();
        let rest = parts.split_off(1);
        let mut acc = parts.pop().expect("chunks yields at least one");
        if per_call {
            t.time_each(&rest, |other| merge(&mut acc, other));
        } else {
            t.time(|| {
                for other in &rest {
                    merge(&mut acc, other);
                }
            });
        }
        std::hint::black_box(&acc);
        RunOutcome {
            work: rest.len() as u64,
            memory_bytes: Some(footprint(&acc) as u64),
            ..Default::default()
        }
    }))
}

/// **Prepare**: fill one, then time the step that makes it ready to answer.
pub fn prepare_measurement<S, I, B, M, Per>(
    metric: Metric,
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
    prepare: fn(&mut S),
) -> Result<Measurement, RunError>
where
    S: 'static,
    I: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
{
    if metric != Metric::Latency {
        return Err(RunError::Sketch(
            "a prepare happens once, so it is timed as one call and nothing else".to_string(),
        ));
    }
    let Sketch { build, footprint } = sketch;
    Ok(Box::new(move |t: &mut Timed| {
        let items = items.as_slice();
        let mut s = build().expect("proven before any measurement ships");
        fill(&insert, &mut s, items);
        t.time(|| prepare(&mut s));
        RunOutcome {
            work: items.len() as u64,
            memory_bytes: Some(footprint(&s) as u64),
            ..Default::default()
        }
    }))
}
