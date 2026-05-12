//! `BenchRunner` — drives a workload through a fresh sketch
//! factory, collects `RunMetrics` per run, aggregates into a
//! `BenchReport`.
//!
//! See `docs/DESIGN.md` §5.3 + §5.4.

use sketch_core::probe::NoopSink;
use sketch_core::report::{Mode, Record, Source};
use sketch_core::sketch::Sketch;
use sketch_core::workload::{Workload, WorkloadDesc};

use crate::accuracy::{Comparison, GroundTruth};
use crate::aggregation::aggregate;
use crate::config::{BenchConfig, MetricsMask};
use crate::metrics::{FullSink, RunMetrics};

/// Drives `config.runs + config.warmup_runs` iterations of a
/// sketch against a fixed workload, feeding each iteration's
/// `Probe<S, FullSink>` output into per-run `RunMetrics`
/// records.
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

    /// Run the bench. `factory` is called once per iteration
    /// (warm-up + measured) to produce a fresh sketch — each run
    /// must see independent state or the aggregated CI is
    /// meaningless. `ground_truth` is optional; when `None` the
    /// accuracy field is left off.
    pub fn run<S, F, G>(&self, mut factory: F, ground_truth: Option<&G>) -> BenchReport
    where
        S: Sketch<Item = W::Item>,
        W::Item: Clone,
        F: FnMut() -> S,
        G: GroundTruth<S>,
    {
        let items = self.workload.items();
        let mut per_run: Vec<RunMetrics> = Vec::with_capacity(self.config.runs);
        let total_runs = self.config.runs + self.config.warmup_runs;

        for run_idx in 0..total_runs {
            let sink = FullSink::new(self.config.metrics);
            let (metrics, final_sketch) = run_once(&mut factory, sink, items, &self.config);

            if run_idx >= self.config.warmup_runs {
                let mut metrics = metrics;
                if self.config.metrics.contains(MetricsMask::ACCURACY) {
                    if let Some(gt) = ground_truth {
                        let cmp = gt.compare(&final_sketch, items);
                        // Query phase ran inside the comparator; pull
                        // its timing into the run's metrics so
                        // aggregation surfaces query_throughput
                        // alongside insertion throughput.
                        metrics.queries_executed = cmp.queries;
                        metrics.query_wall_time_ns = cmp.query_wall_ns;
                        metrics.accuracy = Some(cmp.json);
                    }
                }
                metrics.memory_bytes = Some(final_sketch.memory_bytes() as u64);
                per_run.push(metrics);
            }
        }

        let bench = aggregate(&per_run);
        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.desc(),
            per_run,
            bench,
            config: self.config.clone(),
        }
    }
}

/// One measured run: install `FullSink`, time insert (+
/// optional query phase), return finalized metrics + the
/// underlying sketch (for ground-truth comparison).
///
/// Heap-track windowing (when the `heap-track` feature is on):
/// the `before` snapshot is taken *after* `items` is already
/// allocated by the caller (`workload.items()`) but *before*
/// `factory()` runs, so the sketch's constructor allocations are
/// attributed even for sketches that allocate everything up front
/// (CMS, CountSketch, fixed-matrix HLL). `reset_peak` pins the
/// watermark to that baseline before construction begins.
fn run_once<S, F>(
    factory: &mut F,
    mut sink: FullSink,
    items: &[S::Item],
    config: &BenchConfig,
) -> (RunMetrics, S)
where
    S: Sketch,
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
        use sketch_core::probe::Probe;
        let mut probe: Probe<S, &mut FullSink> = Probe::new(factory_sketch, &mut sink);
        for it in items {
            probe.update(it);
        }
        let (s, _sink) = probe.into_parts();
        s
    };
    // Bill any deferred build/sort/finalize cost to the insert
    // phase so query-throughput numbers measure steady-state
    // queries on a ready-to-answer sketch (see Sketch trait doc).
    sketch.finalize_for_query();
    sink.end_insert_phase();

    #[cfg(feature = "heap-track")]
    let heap_after = crate::metrics::heap_track::snapshot();

    // Query-phase timing is handled by each GroundTruth
    // comparator (which knows the wrapper's natural Query type).
    // A future scalar-only query microbench can live inside the
    // runner, but v1 keeps the runner focused on the insert
    // path — query accuracy + timing both belong to accuracy
    // comparators where the shape is family-specific.
    let _ = config.query_count;

    let memory_bytes = sketch.memory_bytes() as u64;
    #[allow(unused_mut)]
    let mut metrics = sink.finalize(Some(memory_bytes));

    #[cfg(feature = "heap-track")]
    {
        metrics.heap_bytes_net =
            Some((heap_after.in_use - heap_before.in_use).max(0) as u64);
        metrics.heap_bytes_peak =
            Some((heap_after.peak - heap_before.in_use).max(0) as u64);
    }

    (metrics, sketch)
}

/// Output of a `BenchRunner::run`. Convertible to the v1 JSONL
/// record defined in `sketch-core`.
#[derive(Debug, Clone)]
pub struct BenchReport {
    pub sketch: String,
    pub impl_name: String,
    pub workload: WorkloadDesc,
    pub per_run: Vec<RunMetrics>,
    pub bench: sketch_core::report::BenchSection,
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

/// Run a bench without a `GroundTruth`. Helper that pins the
/// `G` parameter to a zero-sized marker so the main API stays
/// generic but callers who don't want accuracy don't have to
/// invent a type.
pub fn run_without_accuracy<S, W, F>(
    cfg: BenchConfig,
    workload: &W,
    sketch_name: impl Into<String>,
    impl_name: impl Into<String>,
    factory: F,
) -> BenchReport
where
    W: Workload,
    W::Item: Clone,
    S: Sketch<Item = W::Item>,
    F: FnMut() -> S,
{
    let runner = BenchRunner::new(cfg, workload, sketch_name, impl_name);
    runner.run::<S, F, NoGT>(factory, None)
}

/// Placeholder `GroundTruth` used when `run_without_accuracy`
/// supplies `None`. Never called; `compare` is a safe default.
pub struct NoGT;
impl<S: Sketch> GroundTruth<S> for NoGT {
    fn compare(&self, _: &S, _: &[S::Item]) -> Comparison {
        Comparison::default()
    }
}

// Suppress unused-import warning when the runner compiles
// without the heap-jemalloc path.
#[allow(dead_code)]
fn _noop() -> NoopSink {
    NoopSink
}
