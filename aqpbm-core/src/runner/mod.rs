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
use crate::accuracy::{Comparison, GroundTruth};
use crate::aggregation::aggregate;
use crate::aggregation::welford::Welford;
use crate::hot_loop::{insert_loop, warmup_cpu_once};
use crate::memory_footprint::MemoryFootprint;
use crate::metrics::{
    CpuTimeSampler, FullSink, ItemsPerSec, JemallocAllocated, MetricsMask, Rss, RunMetrics,
    WallClock,
};
use crate::report::{BenchSection, Mode, Record, RunStats, Source};
use crate::workload::{Workload, WorkloadDescription};

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

    /// The **timed** passes — throughput, latency, merge, plus CPU/MEMORY bits;
    /// accuracy runs in [`run_accuracy`](Self::run_accuracy), off the hot loop.
    /// Each primary bit gets its own report over a fresh population.
    pub fn run_timed<S, F, Insert>(&self, mut factory: F, mut insert: Insert) -> Vec<BenchReport>
    where
        S: Accumulator<Item = W::Item> + MemoryFootprint,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
    {
        let passes = self.config.metrics.passes();
        if passes.is_empty() {
            return Vec::new();
        }
        warmup_cpu_once();
        let mut reports = Vec::with_capacity(passes.len());
        for pass_mask in passes {
            // Accuracy needs the ground truth; it belongs to `run_accuracy`.
            if pass_mask.contains(MetricsMask::ACCURACY) {
                continue;
            }
            let mut pass_cfg = self.config.clone();
            pass_cfg.metrics = pass_mask;
            if pass_mask.contains(MetricsMask::MERGE) {
                // Merge lives on *both* sides: the fold is timed here, its
                // post-merge correctness scored in `run_accuracy`. Folding a
                // single shard measures nothing, so skip it.
                if pass_cfg.merge_shards < 2 {
                    continue;
                }
                reports.push(self.run_merge_pass::<S, _, NoGT, _>(
                    &mut factory,
                    &mut insert,
                    None,
                    pass_cfg,
                ));
            } else if pass_mask == MetricsMask::THROUGHPUT {
                // A throughput pass with no CPU/MEMORY bits takes a slim path
                // that skips RunMetrics and the snapshots. Both paths time the
                // insert loop and `prepare` alike, so no throughput column moves.
                reports.push(self.run_throughput_pass_with(&mut factory, &mut insert, pass_cfg));
            } else {
                // `NoGT` + `None`: these passes ignore ground truth. Naming the
                // type here is what keeps `G` off the public signature.
                reports.push(self.run_pass::<S, _, NoGT, _>(
                    &mut factory,
                    &mut insert,
                    None,
                    pass_cfg,
                ));
            }
        }
        reports
    }

    /// The **accuracy** passes — the accuracy pass itself, and the merge pass
    /// (whose headline output is post-merge accuracy). Untimed relative to the
    /// hot loop, so carrying `G` here costs the timed numbers nothing.
    pub fn run_accuracy<S, F, G, Insert>(
        &self,
        mut factory: F,
        mut insert: Insert,
        gt: &G,
    ) -> Vec<BenchReport>
    where
        S: Accumulator<Item = W::Item> + MemoryFootprint,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
        G: GroundTruth<S>,
    {
        let passes = self.config.metrics.passes();
        if passes.is_empty() {
            return Vec::new();
        }
        warmup_cpu_once();
        let mut reports = Vec::new();
        for pass_mask in passes {
            let mut pass_cfg = self.config.clone();
            pass_cfg.metrics = pass_mask;
            if pass_mask.contains(MetricsMask::MERGE) {
                // `MetricsMask::all()` sets the MERGE bit, so the guard belongs
                // here: asking for "all metrics" must not silently acquire a
                // pass with no shards to fold.
                if pass_cfg.merge_shards < 2 {
                    continue;
                }
                let mut report = self.run_merge_pass(&mut factory, &mut insert, Some(gt), pass_cfg);
                // `run_timed` owns merge timing; this half keeps only the
                // post-merge accuracy, so the two never both claim a
                // `merge_time_ms` — this fold only produces a sketch to score.
                report.bench.merge_time_ms = None;
                reports.push(report);
            } else if pass_mask.contains(MetricsMask::ACCURACY) {
                reports.push(self.run_pass(&mut factory, &mut insert, Some(gt), pass_cfg));
            }
        }
        reports
    }

    /// Throughput-only fast path; the insert closure monomorphizes at the
    /// *caller's* crate so LLVM folds the wrapper's update into the hot loop.
    /// `prepare` runs outside the timed loop, on its own clock.
    #[inline(always)]
    fn run_throughput_pass_with<S, F, Insert>(
        &self,
        factory: &mut F,
        // `&mut Insert`, not `Insert`: the other path reaches `insert_loop`
        // through one `&mut` too, and taking it by value here would instantiate
        // two different `insert_loop` types that only fold thanks to LTO.
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
        let n_items = items.len() as u64;
        let total = pass_cfg.runs + pass_cfg.warmup_runs;
        let mut ns_list: Vec<(u64, u64)> = Vec::with_capacity(pass_cfg.runs);

        for trial in 0..total {
            let mut sketch = factory();
            let ns = insert_loop(&mut sketch, items, insert);
            // Outside `insert_loop` by construction — it is the one function
            // that defines what an insert costs, and nothing else may enter
            // its timed region. See `hot_loop::insert_loop`.
            let finalize_wall = WallClock::start();
            sketch.prepare();
            std::hint::black_box(&sketch);
            let finalize_ns = finalize_wall.elapsed_ns();
            if trial >= pass_cfg.warmup_runs {
                ns_list.push((ns, finalize_ns));
            }
        }

        let mut w = Welford::new();
        let mut build_w = Welford::new();
        let mut fin_w = Welford::new();
        let mut samples: Vec<f64> = Vec::with_capacity(ns_list.len());
        for &(ns, finalize_ns) in &ns_list {
            if ns > 0 {
                let v = ItemsPerSec::compute(n_items, ns);
                w.push(v);
                samples.push(v);
                // Same guard as the ingest column so both summarise the same
                // set of runs — see `aggregation::aggregate`, which this path
                // deliberately mirrors rather than reimplements differently.
                build_w.push(ItemsPerSec::compute(
                    n_items,
                    ns.saturating_add(finalize_ns),
                ));
                fin_w.push(finalize_ns as f64 / 1_000_000.0);
            }
        }
        // No `ci95` on any of these: they are iterations of one process, not
        // independent samples of this implementation. See `RunStats::ci95`.
        let stats = |w: Welford| {
            if w.n() == 0 {
                None
            } else {
                Some(RunStats {
                    mean: w.mean(),
                    stddev: w.stddev(),
                    ci95: None,
                    n: w.n(),
                })
            }
        };
        let throughput = stats(w);
        let build_throughput = stats(build_w);
        let finalize_time_ms = stats(fin_w);
        let throughput_samples = if samples.is_empty() {
            None
        } else {
            Some(samples)
        };

        let bench = BenchSection {
            pass: pass_cfg.metrics.pass_name().map(str::to_string),
            throughput_items_per_sec: throughput,
            throughput_samples,
            build_throughput_items_per_sec: build_throughput,
            finalize_time_ms,
            query_throughput_items_per_sec: None,
            latency_ns: None,
            cpu_time_ms: None,
            wall_time_ms: None,
            rss_peak_kb: None,
            heap_allocated_kb: None,
            memory_bytes: None,
            accuracy: None,
            merge_time_ms: None,
            merge_shards: None,
            merge_supported: None,
        };

        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.description(),
            per_run: Vec::new(),
            bench,
            config: pass_cfg,
        }
    }

    /// Build `merge_shards` sketches over contiguous slices, fold them into one,
    /// and compare against the whole stream. Only the fold is timed. Linear
    /// sketches merge losslessly, so a gap is a defect; KLL's gap is the point.
    fn run_merge_pass<S, F, G, Insert>(
        &self,
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
        let items = self.workload.items();
        let shards = pass_cfg.merge_shards.max(2);
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

            // Warm the query path before the one measured comparison: otherwise
            // the first comparator call is the first query ever issued against
            // this sketch, and `query_throughput` measures a cold path.
            if run_idx < pass_cfg.warmup_runs {
                if let Some(gt) = ground_truth {
                    std::hint::black_box(gt.compare(&acc, items));
                }
            }

            if run_idx >= pass_cfg.warmup_runs {
                merge_ns.push(ns);
                let mut metrics = RunMetrics {
                    items_inserted: items.len() as u64,
                    memory_bytes: Some(acc.memory_bytes() as u64),
                    ..RunMetrics::empty()
                };
                // Accuracy attaches on the first measured run only: this loop
                // re-folds the **same** draw, so N copies would claim N
                // independent draws. Later runs still time `merge_time_ms`.
                if let (Some(gt), true) = (ground_truth, per_run.is_empty()) {
                    let merged = gt.compare(&acc, items);
                    metrics.queries_executed = merged.queries;
                    metrics.query_wall_time_ns = merged.query_wall_ns;

                    // Is merging lossless here? Only meaningful within one
                    // draw, so the single-pass reference is built over the same
                    // items. "Lossless" = indistinguishable on this probe set.
                    let mut single = factory();
                    for it in items {
                        insert(&mut single, it);
                    }
                    single.prepare();
                    let reference = gt.compare(&single, items);
                    // A non-finite metric would compare unequal to itself and
                    // pin this to "lossy" forever, so treat it as unknown
                    // rather than silently reporting a false negative.
                    let comparable = merged
                        .metrics
                        .values()
                        .chain(reference.metrics.values())
                        .all(|v| v.is_finite());
                    let lossless = comparable && merged.metrics == reference.metrics;

                    let mut m = merged.metrics;
                    if comparable {
                        m.insert("merge_lossless".into(), if lossless { 1.0 } else { 0.0 });
                    }
                    metrics.accuracy = Some(m);
                }
                per_run.push(metrics);
            }
        }

        // The pass mask carries only MERGE, and `aggregate` suppresses metrics
        // whose bit is absent. Add ACCURACY when a comparator actually ran, or
        // post-merge accuracy — the point of this pass — is computed then lost.
        let agg_mask = if ground_truth.is_some() {
            pass_cfg.metrics | MetricsMask::ACCURACY
        } else {
            pass_cfg.metrics
        };
        let mut bench = aggregate(&per_run, agg_mask);
        // `agg_mask` says what to aggregate, not which pass ran — it carries the
        // borrowed ACCURACY bit. Restate the pass mask, or the record claims the
        // accuracy pass; those are different runs, and consumers group by this.
        bench.pass = pass_cfg.metrics.pass_name().map(str::to_string);
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
        let accuracy_pass = pass_cfg.metrics.contains(MetricsMask::ACCURACY);

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

            let (metrics, final_sketch) = if pass_cfg.metrics.contains(MetricsMask::LATENCY) {
                // The latency pass deliberately does NOT use `insert`: its
                // instrument *is* the per-update `Probe` boundary. `aggregate`
                // suppresses throughput here because the mask lacks the bit.
                let sink = FullSink::new(pass_cfg.metrics);
                run_once(factory, sink, items, &pass_cfg)
            } else {
                run_once_clean(factory, insert, items, &pass_cfg)
            };

            // The comparator owns the query phase, so it runs on warm-up
            // iterations too — otherwise `--warmup-runs` protects only the
            // insert side and the first measured query is a cold one.
            let comparison = if accuracy_pass {
                ground_truth.map(|gt| gt.compare(&final_sketch, items))
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

        let bench = aggregate(&per_run, pass_cfg.metrics);
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
fn run_once<S, F>(
    factory: &mut F,
    mut sink: FullSink,
    items: &[S::Item],
    config: &BenchConfig,
) -> (RunMetrics, S)
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

    // Query timing belongs to the `GroundTruth` comparators, which know the
    // algorithm-specific query shape; `query_count` is unread as a result.
    let _ = config.query_count;

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

/// Placeholder `GroundTruth` for catalog rows that run without a
/// comparator. Never called; `compare` is a safe default.
pub struct NoGT;
impl<S: Accumulator> GroundTruth<S> for NoGT {
    fn compare(&self, _: &S, _: &[S::Item]) -> Comparison {
        Comparison::default()
    }
}
