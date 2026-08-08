//! `BenchRunner` — drives a workload through a fresh sketch
//! factory, collects `RunMetrics` per run, aggregates into a
//! `BenchReport`.
//!
//! See `docs/DESIGN.md` §5.3 + §5.4.

use std::time::Instant;

pub mod config;

pub use config::BenchConfig;

// Warm-up and the timed insert loop are generic over `Accumulator` and carry no
// sketch-domain knowledge. `insert_loop` stays `#[inline(always)]`, so thin LTO
// folds the wrapper's `update` in across the crate boundary. See `hot_loop`.
use crate::accumulator::Accumulator;
use crate::cell::RunError;
use crate::accuracy::{run_probes, GroundTruth};
use crate::aggregation as fold;
use crate::aggregation::welford::Welford;
use crate::hot_loop::{insert_loop, warmup_cpu_once};
use crate::memory_footprint::MemoryFootprint;
use crate::metrics::{
    cells, Cell, CpuTimeSampler, FullSink, JemallocAllocated, Metric, MetricsMask, Operation,
    OperationMask, Rss, RunMetrics, WallClock,
};
use crate::report::{BenchSection, Mode, Record, RunStats, Source};
use crate::workload::{Workload, WorkloadDescription};


/// Assemble one square's record. The match says what the square produces; the
/// folding functions know nothing about squares, and everything below it rides
/// along with all of them.
fn section(cell: Cell, runs: &[RunMetrics]) -> BenchSection {
    use Metric::{Accuracy, Latency, Throughput};
    use Operation::{Insert, Merge, Prepare, Query};

    let mut bench = BenchSection {
        metric: Some(cell.metric.name().to_string()),
        operation: Some(cell.operation.name().to_string()),
        ..Default::default()
    };
    match (cell.operation, cell.metric) {
        (Insert, Throughput) => {
            bench.throughput_items_per_sec = fold::throughput(runs);
            bench.throughput_samples = fold::throughput_samples(runs);
            // The deferred build's two columns ride on the insert record for
            // want of squares of their own.
            bench.build_throughput_items_per_sec = fold::build_throughput(runs);
            bench.finalize_time_ms = fold::finalize_time_ms(runs);
        }
        (Insert, Latency) => bench.latency_ns = fold::latency_from_recorder(runs),
        (Query, Throughput) => bench.query_throughput_items_per_sec = fold::query_throughput(runs),
        (Query, Latency) => bench.latency_ns = fold::latency_from_calls(runs),
        (Query, Accuracy) => bench.accuracy = fold::accuracy(runs),
        // The fold's own timing is filled by `run_merge_pass`, which is what
        // holds the clock around it.
        (Merge, Latency) => {}
        (Insert, Accuracy)
        | (Merge, Accuracy)
        | (Merge, Throughput)
        | (Prepare, Throughput | Latency | Accuracy) => {}
    }

    // Phase-boundary readings contaminate nothing, so they ride along with
    // every square.
    if cell.secondary.contains(MetricsMask::CPU) {
        bench.cpu_time_ms = fold::cpu_time_ms(runs);
    }
    if cell.secondary.contains(MetricsMask::MEMORY) {
        let m = fold::memory_maxima(runs);
        bench.rss_peak_kb = m.rss_peak_kb;
        bench.heap_allocated_kb = m.heap_allocated_kb;
        bench.heap_bytes_net = m.heap_bytes_net;
        bench.heap_bytes_peak = m.heap_bytes_peak;
    }
    bench.wall_time_ms = fold::wall_time_ms(runs);
    bench.memory_bytes = fold::memory_bytes(runs);
    bench
}

/// Whether a request reaches any square that cannot run without a comparator.
/// Wider than "measures accuracy": issuing the queries is what a comparator
/// does, so every square over the query operation needs one.
///
/// A caller builds a comparator when this says to, instead of being asked.
pub fn needs_ground_truth(operations: OperationMask, metrics: MetricsMask) -> bool {
    cells(operations, metrics)
        .into_iter()
        .any(|c| c.operation == Operation::Query)
}

/// A square the grid admits and nothing measures.
fn unmeasured(cell: Cell) -> RunError {
    RunError::NotMeasured {
        operation: cell.operation.name(),
        metric: cell.metric.name(),
    }
}

/// Drives `config.runs + config.warmup_runs` iterations against a fixed
/// workload, feeding each `Probe<S, FullSink>` into per-run `RunMetrics`.
pub struct BenchRunner<'a, W: Workload> {
    config: BenchConfig,
    workload: &'a W,
    sketch_name: String,
    impl_name: String,
}

impl<'a, W: Workload> BenchRunner<'a, W> {
    pub fn new(
        config: BenchConfig,
        workload: &'a W,
        sketch_name: impl Into<String>,
        impl_name: impl Into<String>,
    ) -> Self {
        Self {
            config,
            workload,
            sketch_name: sketch_name.into(),
            impl_name: impl_name.into(),
        }
    }

    /// Run every square the request selects. One place, one match: each
    /// square of the grid either names the call that measures it or says
    /// nothing measures it.
    ///
    /// `ground_truth` is what a square needing a comparator gets. Passing
    /// `None` runs the squares that need none and skips the rest, which is
    /// what the timed half of a cell does.
    pub fn run<S, F, G, Insert>(
        &self,
        mut factory: F,
        mut insert: Insert,
        ground_truth: Option<&G>,
    ) -> Result<Vec<BenchReport>, RunError>
    where
        S: Accumulator<Item = W::Item> + MemoryFootprint,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
        G: GroundTruth<S>,
    {
        use Metric::{Accuracy, Latency, Throughput};
        use Operation::{Insert as Ins, Merge, Prepare, Query};

        let cells = cells(self.config.operations, self.config.metrics);
        if cells.is_empty() {
            return Ok(Vec::new());
        }
        warmup_cpu_once();
        let mut reports = Vec::with_capacity(cells.len());
        for cell in cells {
            let mut pass_cfg = self.config.clone();
            pass_cfg.metrics = cell.mask();
            // The grid. No `_` arm: adding an operation or a metric fails to
            // compile until the new squares say what they measure.
            let report = match (cell.operation, cell.metric) {
                (Ins, Throughput) | (Ins, Latency) => {
                    Some(self.run_pass::<S, _, G, _>(cell, &mut factory, &mut insert, None, pass_cfg))
                }
                (Ins, Accuracy) => return Err(unmeasured(cell)),
                // Issuing the queries is the measurement, so this square
                // needs the comparator as much as accuracy does.
                (Query, Throughput) => ground_truth
                    .map(|gt| self.run_pass(cell, &mut factory, &mut insert, Some(gt), pass_cfg)),
                // Same loop as query throughput, with the clock inside it.
                (Query, Latency) => ground_truth
                    .map(|gt| self.run_pass(cell, &mut factory, &mut insert, Some(gt), pass_cfg)),
                (Query, Accuracy) => ground_truth
                    .map(|gt| self.run_pass(cell, &mut factory, &mut insert, Some(gt), pass_cfg)),
                (Merge, Throughput) => return Err(unmeasured(cell)),
                (Merge, Latency) => {
                    if pass_cfg.merge_shards < 2 {
                        return Err(RunError::NothingToFold {
                            shards: pass_cfg.merge_shards,
                        });
                    }
                    Some(self.run_merge_pass::<S, _, _>(cell, &mut factory, &mut insert, pass_cfg))
                }
                // Scoring a folded sketch means querying it, which is the
                // query operation wearing merge's name. Merge itself produces
                // no answer to be right or wrong about.
                (Merge, Accuracy) => return Err(unmeasured(cell)),
                (Prepare, Throughput) => return Err(unmeasured(cell)),
                (Prepare, Latency) => return Err(unmeasured(cell)),
                (Prepare, Accuracy) => return Err(unmeasured(cell)),
            };
            if let Some(report) = report {
                reports.push(report);
            }
        }
        Ok(reports)
    }

    /// Build `merge_shards` sketches over contiguous slices, fold them into one,
    /// and compare against the whole stream. Only the fold is timed. Linear
    /// sketches merge losslessly, so a gap is a defect; KLL's gap is the point.
    fn run_merge_pass<S, F, Insert>(
        &self,
        cell: Cell,
        factory: &mut F,
        insert: &mut Insert,
        pass_cfg: BenchConfig,
    ) -> BenchReport
    where
        S: Accumulator<Item = W::Item> + MemoryFootprint,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
    {
        let items = self.workload.items();
        // No clamp: the guard above refused anything below two, so this is the
        // number the caller asked for and the number the record will report.
        let shards = pass_cfg.merge_shards;
        let mut per_run: Vec<RunMetrics> = Vec::with_capacity(pass_cfg.runs);
        let mut merge_ns: Vec<u64> = Vec::new();
        let mut supported = true;
        let mut folded_shards = shards;

        for run_idx in 0..(pass_cfg.runs + pass_cfg.warmup_runs) {
            // Contiguous partition. `chunks` leaves the last shard short when
            // the split is uneven; that is a property of the data, not of the
            // merge, and rounding it away would misreport shard count.
            let per_shard = items.len().div_ceil(shards);
            let mut sketches: Vec<S> = items
                .chunks(per_shard.max(1))
                .map(|chunk| {
                    let mut sk = factory();
                    for it in chunk {
                        insert(&mut sk, it);
                    }
                    sk
                })
                .collect();

            // `chunks` yields ceil(n / per_shard) pieces, often strictly fewer
            // than requested. Reporting the request would put merge cost against
            // the wrong x, and a short stream could claim a fold that never ran.
            let actual_shards = sketches.len();
            if actual_shards < 2 {
                supported = false;
                break;
            }
            let mut acc = sketches.remove(0);
            let start = Instant::now();
            for other in &sketches {
                if acc.merge(other).is_err() {
                    supported = false;
                    break;
                }
            }
            let ns = start.elapsed().as_nanos() as u64;
            if !supported {
                break;
            }
            folded_shards = actual_shards;
            std::hint::black_box(&acc);
            acc.prepare();

            if run_idx >= pass_cfg.warmup_runs {
                merge_ns.push(ns);
                per_run.push(RunMetrics {
                    items_inserted: items.len() as u64,
                    memory_bytes: Some(acc.memory_bytes() as u64),
                    ..RunMetrics::empty()
                });
            }
        }

        let mut bench = section(cell, &per_run);
        bench.operation = Some(cell.operation.name().to_string());
        bench.metric = Some(cell.metric.name().to_string());
        // The fold's cost belongs to the cell that asked for it. The scored
        // cell folds only to have something to query, so it reports no time.
        if cell.metric != Metric::Latency {
            bench.merge_time_ms = None;
        }
        // Nothing in this pass is timed end-to-end: shard filling is
        // deliberately excluded and only the fold is measured, so a
        // `wall_time_ms` of 0 would claim a measurement that was not taken.
        bench.wall_time_ms = None;
        bench.merge_shards = Some(folded_shards);
        bench.merge_supported = Some(supported);
        if supported && !merge_ns.is_empty() {
            let mut w = Welford::new();
            for ns in &merge_ns {
                w.push(*ns as f64 / 1_000_000.0);
            }
            bench.merge_time_ms = Some(RunStats {
                mean: w.mean(),
                stddev: w.stddev(),
                ci95: None,
                n: w.n(),
            });
        }

        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.description(),
            per_run,
            bench,
            config: pass_cfg,
        }
    }

    fn run_pass<S, F, G, Insert>(
        &self,
        cell: Cell,
        factory: &mut F,
        insert: &mut Insert,
        ground_truth: Option<&G>,
        pass_cfg: BenchConfig,
    ) -> BenchReport
    where
        S: Accumulator<Item = W::Item> + MemoryFootprint,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
        G: GroundTruth<S>,
    {
        // Measuring the query operation means issuing queries, and the
        // comparator is what issues them. Wider than measuring accuracy.
        let queries = cell.operation == Operation::Query;
        // Timing each question on its own is what a latency measurement is,
        // and what a throughput measurement must not pay for. The runner owns
        // the choice because it owns the square.
        let per_call = cell.metric == Metric::Latency;
        // Resampling is an accuracy concern: error is deterministic given
        // (data, parameters), so repeats over one draw would fabricate spread.
        // A timing over the same draw is a real repeat.
        let accuracy_pass = cell.metric == Metric::Accuracy;

        // A repetition is only worth running if it draws its own sample: error
        // is deterministic given (data, parameters), so repeats over one fixed
        // workload fabricate spread. Non-resamplable → one run, `n = 1`.
        let measured_runs = if accuracy_pass && !self.workload.can_resample() {
            1
        } else {
            pass_cfg.runs
        };
        let mut per_run: Vec<RunMetrics> = Vec::with_capacity(measured_runs);
        let total_runs = measured_runs + pass_cfg.warmup_runs;

        for run_idx in 0..total_runs {
            // Warm-ups reuse the base workload (their results are discarded,
            // so generating a fresh one would be pure cost); measured run 0
            // uses it too, and each later measured run draws its own.
            let measured_idx = run_idx.checked_sub(pass_cfg.warmup_runs);
            let resampled: Option<W> = match measured_idx {
                Some(j) if accuracy_pass && j > 0 => self.workload.resample(j),
                _ => None,
            };
            let workload: &W = resampled.as_ref().unwrap_or(self.workload);
            let items = workload.items();

            let (metrics, final_sketch) = if cell.metric == Metric::Latency {
                // The latency pass deliberately does NOT use `insert`: its
                // instrument *is* the per-update `Probe` boundary. `aggregate`
                // suppresses throughput here because the mask lacks the bit.
                let sink = FullSink::new(pass_cfg.metrics);
                run_once(factory, sink, items)
            } else {
                run_once_clean(factory, insert, items, &pass_cfg)
            };

            // The comparator owns the query phase, so it runs on warm-up
            // iterations too — otherwise `--warmup-runs` protects only the
            // insert side and the first measured query is a cold one.
            let comparison = if queries {
                ground_truth.map(|gt| run_probes(gt, &final_sketch, items, per_call))
            } else {
                None
            };

            if run_idx >= pass_cfg.warmup_runs {
                let mut metrics = metrics;
                if let Some(cmp) = comparison {
                    // Query phase ran inside the comparator; pull its timing
                    // into the run's metrics so aggregation surfaces
                    // query_throughput alongside insertion throughput.
                    metrics.queries_executed = cmp.queries;
                    metrics.query_wall_time_ns = cmp.query_wall_ns;
                    metrics.accuracy = Some(cmp.metrics);
                    metrics.query_calls = cmp.query_calls;
                }
                metrics.memory_bytes = Some(final_sketch.memory_bytes() as u64);
                per_run.push(metrics);
            }
        }

        let mut bench = section(cell, &per_run);
        bench.operation = Some(cell.operation.name().to_string());
        bench.metric = Some(cell.metric.name().to_string());
        // `runs` says how many were measured, not asked for: a non-resamplable
        // workload measures once, and `runs: 10` beside `accuracy_runs: 1` is
        // self-contradictory and inflates any confidence proxy taken from it.
        let mut pass_cfg = pass_cfg;
        pass_cfg.runs = per_run.len();
        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.description(),
            per_run,
            bench,
            config: pass_cfg,
        }
    }
}

/// One measured run through `Probe<S, FullSink>`: time the insert phase, return
/// metrics plus sketch. Under `heap-track` the `before` snapshot lands after
/// `items` but before `factory()`, so constructor allocations are attributed.
fn run_once<S, F>(factory: &mut F, mut sink: FullSink, items: &[S::Item]) -> (RunMetrics, S)
where
    S: Accumulator + MemoryFootprint,
    S::Item: Clone,
    F: FnMut() -> S,
{
    sink.on_run_start();

    #[cfg(feature = "heap-track")]
    let heap_before = {
        crate::metrics::heap_track::reset_peak();
        crate::metrics::heap_track::snapshot()
    };

    let factory_sketch = factory();

    // Insert phase.
    sink.begin_insert_phase();
    let mut sketch = {
        use crate::probe::Probe;
        let mut probe: Probe<S, &mut FullSink> = Probe::new(factory_sketch, &mut sink);
        for it in items {
            probe.update(it);
        }
        let (s, _sink) = probe.into_parts();
        s
    };
    // Run any deferred build before the query phase, so query throughput
    // measures a ready-to-answer sketch. Timed on its own clock — folding it
    // into insert would change what `insert_wall_time_ns` means on this path.
    let finalize_wall = WallClock::start();
    sketch.prepare();
    let finalize_wall_time_ns = finalize_wall.elapsed_ns();
    sink.end_insert_phase();

    #[cfg(feature = "heap-track")]
    let heap_after = crate::metrics::heap_track::snapshot();

    let memory_bytes = sketch.memory_bytes() as u64;
    let mut metrics = sink.finalize(Some(memory_bytes));
    metrics.finalize_wall_time_ns = finalize_wall_time_ns;

    #[cfg(feature = "heap-track")]
    {
        metrics.heap_bytes_net = Some((heap_after.in_use - heap_before.in_use).max(0) as u64);
        metrics.heap_bytes_peak = Some((heap_after.peak - heap_before.in_use).max(0) as u64);
    }

    (metrics, sketch)
}

/// One measured run with no per-update instrumentation, for the THROUGHPUT and
/// ACCURACY passes: no `Probe` wrapper, so the sketch's `update` is alone in the
/// loop. Phase-boundary metrics still attach via direct primitives.
#[inline(always)]
fn run_once_clean<S, F, Insert>(
    factory: &mut F,
    insert: &mut Insert,
    items: &[S::Item],
    config: &BenchConfig,
) -> (RunMetrics, S)
where
    S: Accumulator + MemoryFootprint,
    S::Item: Clone,
    F: FnMut() -> S,
    Insert: FnMut(&mut S, &S::Item),
{
    let wall = WallClock::start();
    let mut cpu = if config.metrics.contains(MetricsMask::CPU) {
        Some(CpuTimeSampler::start())
    } else {
        None
    };

    #[cfg(feature = "heap-track")]
    let heap_before = {
        crate::metrics::heap_track::reset_peak();
        crate::metrics::heap_track::snapshot()
    };

    let mut sketch = factory();

    // Same `insert_loop` the throughput fast path uses, driving the same
    // caller-supplied closure — the two paths must not be able to disagree
    // about what an insert costs.
    let insert_wall_time_ns = insert_loop(&mut sketch, items, insert);
    let finalize_wall = WallClock::start();
    sketch.prepare();
    std::hint::black_box(&sketch);
    let finalize_wall_time_ns = finalize_wall.elapsed_ns();

    #[cfg(feature = "heap-track")]
    let heap_after = crate::metrics::heap_track::snapshot();

    let wall_time_ns = wall.elapsed_ns();
    let (cpu_user_ns, cpu_sys_ns) = match cpu.take() {
        Some(sampler) => {
            let s = sampler.finish();
            (Some(s.user_ns), Some(s.sys_ns))
        }
        None => (None, None),
    };
    let (rss_peak_kb, heap_allocated_kb) = if config.metrics.contains(MetricsMask::MEMORY) {
        (Rss::peak_kb(), JemallocAllocated::read_kb())
    } else {
        (None, None)
    };

    let memory_bytes = sketch.memory_bytes() as u64;

    #[allow(unused_mut)]
    let mut metrics = RunMetrics {
        items_inserted: items.len() as u64,
        queries_executed: 0,
        wall_time_ns,
        insert_wall_time_ns,
        finalize_wall_time_ns,
        query_wall_time_ns: 0,
        cpu_user_ns,
        cpu_sys_ns,
        rss_peak_kb,
        heap_allocated_kb,
        memory_bytes: Some(memory_bytes),
        heap_bytes_net: None,
        heap_bytes_peak: None,
        latency_ns: None,
        accuracy: None,
        query_calls: None,
    };

    #[cfg(feature = "heap-track")]
    {
        metrics.heap_bytes_net = Some((heap_after.in_use - heap_before.in_use).max(0) as u64);
        metrics.heap_bytes_peak = Some((heap_after.peak - heap_before.in_use).max(0) as u64);
    }

    (metrics, sketch)
}

/// Output of one `BenchRunner` pass. Convertible to the v1 JSONL [`Record`].
#[derive(Debug, Clone)]
pub struct BenchReport {
    pub sketch: String,
    pub impl_name: String,
    pub workload: WorkloadDescription,
    pub per_run: Vec<RunMetrics>,
    pub bench: crate::report::BenchSection,
    pub config: BenchConfig,
}

impl BenchReport {
    /// Build a v1 JSONL record from this report.
    pub fn to_record(&self) -> Record {
        let mut rec = Record::new(
            self.sketch.clone(),
            self.impl_name.clone(),
            self.workload.clone(),
            Mode::Bench,
            self.config.runs,
        );
        rec.bench = Some(self.bench.clone());
        rec.source = Source::Cli;
        rec
    }

    pub fn to_jsonl(&self) -> String {
        self.to_record().to_jsonl()
    }
}

/// Placeholder `GroundTruth` for rows that run without a comparator. It knows
/// no truth and asks nothing, so a square needing one measures nothing.
pub struct NoGT;
impl<S: Accumulator> GroundTruth<S> for NoGT {
    type Truth = ();
    type Probe = ();
    type Answer = ();

    fn truth(&self, _: &[S::Item]) {}
    fn probes(&self, _: &()) -> Vec<()> {
        Vec::new()
    }
    fn ask(&self, _: &S, _: &()) {}
    fn score(&self, _: &(), _: &[()], _: &[()]) -> std::collections::BTreeMap<String, f64> {
        std::collections::BTreeMap::new()
    }
}
