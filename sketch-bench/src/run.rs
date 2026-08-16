//! How a measurement is performed, once a row has been selected.
//!
//! Each function here builds one **self-contained owning closure per
//! measurement** and hands it to `aqpbm_core::measure`. The closure builds its
//! own sketch, fills it if it needs a filled one, marks the region it wants
//! timed, and reports what it did. Core times it and knows nothing else.
//!
//! Every closure owns its sketch outright, so no two share one and none takes a
//! `&mut S` from outside. That is why a fresh instance per run is free to be the
//! rule: an insert measured ten times starts from empty ten times, and a query
//! measured ten times pays its own fill each time — which matters, because
//! `sketch_oxide`'s KLL sorts lazily and a re-queried sketch is already sorted.
//!
//! Nothing here names a sketch or a row. It is the bridge between core's
//! measure loop and the verbs in `crate::ops`: `registry` says *which* sketch
//! runs, this says *how* one is run.

use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::measure::{MeasureConfig, RunOutcome};
use aqpbm_core::metrics::{cells, Cell, Metric, Operation, RunMetrics};
use aqpbm_core::request::Requirement;
use aqpbm_core::runner::BenchReport;
use aqpbm_core::workload::Workload;
use asap_sketchlib::{DefaultXxHasher, FastPathHasher, MatrixStorage};

use crate::ops::SketchOps;
use crate::params::ParamSet;
use crate::registry::GroundTruthCalculator;
use crate::wrappers::fixed_matrix;

//
// Each of these builds one **self-contained owning closure per measurement**
// and hands it to `aqpbm_core::measure`. The closure builds its own sketch,
// fills it if it needs a filled one, marks the region it wants timed, and
// reports what it did. Core times it and knows nothing else.
//
// Every closure owns its sketch outright, so no two share one and none takes a
// `&mut S` from outside. That is why a fresh instance per run is free to be the
// rule: an insert measured ten times starts from empty ten times, and a query
// measured ten times pays its own fill each time — which matters, because
// `sketch_oxide`'s KLL sorts lazily and a re-queried sketch is already sorted.

/// Run every selected square of one scored row.
///
/// The row materialises its workload once, at its own item type, and every
/// selected square runs off that one materialisation.
pub(crate) fn run_row<S, I, G, Ins>(
    cfg: &MeasureConfig,
    req: &Requirement,
    workload: &impl Workload<Item = I>,
    gt: &G,
    insert: Ins,
    ops: &SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<Vec<BenchReport>, RunError>
where
    I: Clone,
    G: GroundTruth<I>,
    Ins: Fn(&mut S, &I) + Copy,
{
    let items = workload.items();
    // The squares this request selects. A pure function of the request, so
    // asking for them here cannot answer differently from the call `resolve`
    // validated against.
    let squares = cells(req.operations, req.metrics);
    let mut out = Vec::with_capacity(squares.len());
    for &cell in &squares {
        let runs = one_square(cfg, req, cell, items, gt, insert, ops)?;
        let mut report = BenchReport::fold(
            req.algorithm.as_str(),
            req.impl_name.as_str(),
            workload.description(),
            cell.operation,
            cell.metric,
            runs,
        );
        // The count that actually folded, not the one asked for: a request
        // below the floor is raised, and a record states what ran.
        if cell.operation == Operation::Merge {
            report.bench.merge_shards = Some(req.merge_shards.max(MIN_MERGE_SHARDS));
        }
        out.push(report);
    }
    Ok(out)
}

/// One square: build the body, hand it to core.
fn one_square<S, I, G, Ins>(
    cfg: &MeasureConfig,
    req: &Requirement,
    cell: Cell,
    items: &[I],
    gt: &G,
    insert: Ins,
    ops: &SketchOps<S, I, G::Probe, G::Answer>,
) -> Result<Vec<RunMetrics>, RunError>
where
    I: Clone,
    G: GroundTruth<I>,
    Ins: Fn(&mut S, &I) + Copy,
{
    let params = &req.params;
    // A run of no threads is not a run. Floored here rather than trusted from
    // the request, because a library caller builds its own `Requirement`.
    let workers = req.workers.max(1);
    // Refuse here rather than inside the closure: a build that cannot happen is
    // not a measurement of zero.
    (ops.build)(params, workers).map_err(|e| RunError::Body(e.to_string()))?;

    match cell.operation {
        Operation::Insert => Ok(aqpbm_core::measure(cfg, |t| {
            let mut s = (ops.build)(params, workers).expect("proven above");
            match cell.metric {
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
        })),

        Operation::Query => {
            let truth = gt.truth(items);
            let probes = gt.probes(&truth);
            Ok(aqpbm_core::measure(cfg, |t| {
                // Setup: a fresh sketch, filled. Outside the clock, and redone
                // every run so no run inherits another's warmed state.
                let mut s = (ops.build)(params, workers).expect("proven above");
                for v in items {
                    insert(&mut s, v);
                }
                ops.run_prepare(&mut s);

                let mut answers = Vec::with_capacity(probes.len());
                t.time(|| {
                    for p in &probes {
                        answers.push((ops.ask)(&mut s, p));
                    }
                });
                let scores = if cell.metric == Metric::Accuracy {
                    gt.score(&truth, &probes, &answers)
                } else {
                    Default::default()
                };
                RunOutcome {
                    work: probes.len() as u64,
                    memory_bytes: Some((ops.memory)(&s) as u64),
                    scores,
                }
            }))
        }

        Operation::Prepare => Ok(aqpbm_core::measure(cfg, |t| {
            let mut s = (ops.build)(params, workers).expect("proven above");
            for v in items {
                insert(&mut s, v);
            }
            t.time(|| ops.run_prepare(&mut s));
            RunOutcome {
                work: items.len() as u64,
                memory_bytes: Some((ops.memory)(&s) as u64),
                ..Default::default()
            }
        })),

        Operation::Merge => {
            let Some(merge) = ops.merge else {
                return Err(RunError::Body(format!(
                    "{}/{} provides no merge",
                    req.algorithm, req.impl_name
                )));
            };
            let shards = req.merge_shards.max(MIN_MERGE_SHARDS);
            let per = items.len().div_ceil(shards).max(1);
            Ok(aqpbm_core::measure(cfg, |t| {
                // Filling the shards is setup; only the fold is the measurement.
                let mut parts: Vec<S> = items
                    .chunks(per)
                    .map(|chunk| {
                        let mut s = (ops.build)(params, workers).expect("proven above");
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
            }))
        }
    }
}

/// The shape-independent half of a fixed-matrix row.
///
/// The one place a closure cannot be written at the row: the sketch type is a
/// GAT, so there is no single type to write one against. A generic method is
/// the stand-in, and the bodies still live in the wrapper file.
pub trait FixedMatrixRow {
    const ALGORITHM: &'static str;
    type At<
        M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    >;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;
    fn insert<M>(sketch: &mut Self::At<M>, v: &i64)
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
    fn ops<M>() -> SketchOps<Self::At<M>, i64, i64, u64>
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
}

/// Turn the requested `(rows, cols)` back into the monomorphisation that bakes
/// it in, then run every square against that.
pub(crate) fn run_fixed_matrix<W: FixedMatrixRow>(
    cfg: &MeasureConfig,
    req: &Requirement,
    data: WorkloadData,
) -> Result<Vec<BenchReport>, RunError> {
    let (rows, cols) = W::shape(&req.params)?;
    let wk = <i64 as BenchItem>::materialise(data)?;
    let gt = <FrequencyGT as GroundTruthCalculator<i64>>::build(&req.params);
    let visitor = RunFixedMatrix::<W> {
        cfg,
        req,
        wk: &wk,
        gt: &gt,
        _row: std::marker::PhantomData,
    };
    fixed_matrix::with_fixed_matrix(rows, cols, visitor).unwrap_or_else(|| {
        Err(RunError::Body(fixed_matrix::unsupported_shape(
            W::ALGORITHM,
            rows,
            cols,
        )))
    })
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    cfg: &'a MeasureConfig,
    req: &'a Requirement,
    wk: &'a aqpbm_core::workload::NumericWorkload<i64>,
    gt: &'a FrequencyGT,
    _row: std::marker::PhantomData<W>,
}

impl<W: FixedMatrixRow> fixed_matrix::FixedMatrixVisitor for RunFixedMatrix<'_, W> {
    type Out = Result<Vec<BenchReport>, RunError>;
    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        run_row::<W::At<M>, i64, FrequencyGT, _>(
            self.cfg,
            self.req,
            self.wk,
            self.gt,
            W::insert::<M>,
            &W::ops::<M>(),
        )
    }
}

/// The smallest fold that is a merge at all: two shards, one merge call. The
/// count itself comes from the request, and this only keeps a `--merge-shards 1`
/// from measuring an empty loop.
pub(crate) const MIN_MERGE_SHARDS: usize = 2;

/// A row nothing scores: insert and prepare only, no comparator, no probes.
pub(crate) fn run_row_unscored<S, I, Ins>(
    cfg: &MeasureConfig,
    req: &Requirement,
    workload: &impl Workload<Item = I>,
    insert: Ins,
    ops: &SketchOps<S, I, (), ()>,
) -> Result<Vec<BenchReport>, RunError>
where
    I: Clone,
    Ins: Fn(&mut S, &I) + Copy,
{
    run_row::<S, I, NoScore, Ins>(cfg, req, workload, &NoScore, insert, ops)
}

/// The comparator for a row nothing scores. Computes no truth and asks nothing.
pub(crate) struct NoScore;

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
