//! `BenchRunner` — drives a workload through a fresh sketch
//! factory, collects `RunMetrics` per run, aggregates into a
//! `BenchReport`.
//!
//! See `docs/DESIGN.md` §5.3 + §5.4.

use std::sync::Once;
use std::time::{Duration, Instant};

/// Ramp the CPU **once per process**, before the first measured loop of any
/// pass.
///
/// This used to be called from the throughput fast path only, which made the
/// warm-up an accident of which `--metrics` flags were passed: a THROUGHPUT
/// pass that also carried the CPU/MEMORY bits took the other branch and was
/// timed with no governor ramp at all. Two passes measured at two different
/// clock states is not a comparison. Gating on `Once` also stops a sweep from
/// burning the warm-up duration once per (impl, config) pair — for a 63-pair
/// sweep that was ten minutes of spinning for a ramp that only the first pair
/// actually needed.
fn warmup_cpu_once() {
    static WARMED: Once = Once::new();
    WARMED.call_once(warmup_cpu_from_env);
}

/// Burn CPU on the current core so the cpufreq governor ramps to max turbo
/// before timing starts. External shell warmups don't work reliably because
/// the governor can drop frequency during the bench process's exec/startup
/// window.
///
/// Duration is read from `BENCH_WARMUP_SECS`. **Defaults to 0** — a library
/// must not burn ten seconds of a caller's CPU because it was linked. The
/// measurement default lives in `sketchlib`'s `main`, which sets the variable
/// when the operator hasn't; every integration test and downstream embedder
/// therefore pays nothing. (`cfg!(test)` cannot express this: an integration
/// test links this crate as a plain dependency, compiled without `cfg(test)`.)
fn warmup_cpu_from_env() {
    let secs: u64 = std::env::var("BENCH_WARMUP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if secs == 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut x: u64 = 0xdeadbeef;
    while Instant::now() < deadline {
        for _ in 0..10_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        std::hint::black_box(x);
    }
}

use sketch_core::probe::NoopSink;
use sketch_core::report::{BenchSection, Mode, Record, RunStats, Source};
use sketch_core::sketch::Sketch;
use sketch_core::workload::{Workload, WorkloadDesc};

use crate::accuracy::{Comparison, GroundTruth};
use crate::aggregation::aggregate;
use crate::aggregation::welford::Welford;
use crate::config::{BenchConfig, MetricsMask};
use crate::metrics::{
    CpuTimeSampler, FullSink, ItemsPerSec, JemallocAllocated, RunMetrics, Rss, WallClock,
};

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

    /// Run the bench across the metric passes encoded in
    /// `config.metrics`. Each primary bit (THROUGHPUT / LATENCY /
    /// ACCURACY) produces its **own** `BenchReport` with a fresh
    /// sketch population — see [`MetricsMask::passes`]. Returns
    /// one report per pass, in the order produced by `passes()`.
    ///
    /// Rationale for strict per-pass isolation: per-update
    /// instrumentation (latency `Instant::now()` pair) inflates
    /// the insert-phase wall clock, which is the throughput
    /// denominator. Splitting throughput and latency into two
    /// independent passes guarantees the throughput pass sees a
    /// clean hot path. Accuracy gets its own pass too so its
    /// insert phase isn't billed any per-op overhead either.
    ///
    /// `factory` is called once per iteration (warm-up +
    /// measured) **per pass** — each run must see independent
    /// state or the aggregated CI is meaningless.
    /// `ground_truth` is consulted only in the ACCURACY pass.
    /// `insert` is the hot-loop body. It is a caller-supplied closure —
    /// **not** a `sketch.update(it)` call written here — so that it
    /// monomorphizes in the crate that defines the wrapper, letting LLVM
    /// fold the wrapper's `update` into the loop. Every pass that reports
    /// throughput drives this same closure, so the number cannot depend on
    /// which `--metrics` flags were passed. See `insert_loop`.
    pub fn run<S, F, G, Insert>(
        &self,
        mut factory: F,
        mut insert: Insert,
        ground_truth: Option<&G>,
    ) -> Vec<BenchReport>
    where
        S: Sketch<Item = W::Item>,
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
        let mut reports = Vec::with_capacity(passes.len());
        for pass_mask in passes {
            let mut pass_cfg = self.config.clone();
            pass_cfg.metrics = pass_mask;
            // A throughput pass with no secondary CPU/MEMORY bits takes a
            // slim path that skips RunMetrics, CPU/RSS/heap snapshots,
            // finalize_for_query, memory_bytes, and Welford-via-aggregate.
            // Both paths run the identical `insert` closure over the
            // identical slice and time only that loop, so the choice
            // affects what else is collected — never the throughput.
            if pass_mask == MetricsMask::THROUGHPUT {
                reports.push(self.run_throughput_pass_with(&mut factory, &mut insert, pass_cfg));
            } else {
                reports.push(self.run_pass(&mut factory, &mut insert, ground_truth, pass_cfg));
            }
        }
        reports
    }

    /// Throughput-only fast path with an explicit insert closure.
    /// The closure body monomorphizes at the *caller's* crate,
    /// giving LLVM a direct shot at folding the wrapper's update
    /// into the hot loop.
    #[inline(always)]
    fn run_throughput_pass_with<S, F, Insert>(
        &self,
        factory: &mut F,
        // `&mut Insert`, not `Insert`: `run_pass` -> `run_once_clean` also
        // reaches `insert_loop` through one `&mut`, so taking it by value
        // here would instantiate `insert_loop::<_, &mut &mut Insert>` on this
        // path and `insert_loop::<_, &mut Insert>` on the other. They fold
        // today, but only because of `#[inline(always)]` + LTO — the exact
        // mechanism whose failure caused the 5.1% split this fix exists to
        // close. Same type on both paths makes it structural instead.
        insert: &mut Insert,
        pass_cfg: BenchConfig,
    ) -> BenchReport
    where
        S: Sketch<Item = W::Item>,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
    {
        let items = self.workload.items();
        let n_items = items.len() as u64;
        let total = pass_cfg.runs + pass_cfg.warmup_runs;
        let mut ns_list: Vec<u64> = Vec::with_capacity(pass_cfg.runs);

        for trial in 0..total {
            let mut sketch = factory();
            let ns = insert_loop(&mut sketch, items, insert);
            if trial >= pass_cfg.warmup_runs {
                ns_list.push(ns);
            }
        }

        let mut w = Welford::new();
        let mut samples: Vec<f64> = Vec::with_capacity(ns_list.len());
        for &ns in &ns_list {
            if ns > 0 {
                let v = ItemsPerSec::compute(n_items, ns);
                w.push(v);
                samples.push(v);
            }
        }
        let throughput = if w.n() == 0 {
            None
        } else {
            let (lo, hi) = w.ci95();
            Some(RunStats {
                mean: w.mean(),
                stddev: w.stddev(),
                ci95: [lo, hi],
                n: w.n(),
            })
        };
        let throughput_samples = if samples.is_empty() { None } else { Some(samples) };

        let bench = BenchSection {
            throughput_items_per_sec: throughput,
            throughput_samples,
            query_throughput_items_per_sec: None,
            latency_ns: None,
            cpu_time_ms: None,
            wall_time_ms: None,
            rss_peak_kb: None,
            heap_allocated_kb: None,
            memory_bytes: None,
            accuracy: None,
        };

        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.desc(),
            per_run: Vec::new(),
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
        S: Sketch<Item = W::Item>,
        W::Item: Clone,
        F: FnMut() -> S,
        Insert: FnMut(&mut S, &W::Item),
        G: GroundTruth<S>,
    {
        let items = self.workload.items();
        let mut per_run: Vec<RunMetrics> = Vec::with_capacity(pass_cfg.runs);
        let total_runs = pass_cfg.runs + pass_cfg.warmup_runs;

        for run_idx in 0..total_runs {
            let (metrics, final_sketch) = if pass_cfg.metrics.contains(MetricsMask::LATENCY) {
                // The latency pass deliberately does NOT use `insert`: its
                // instrument *is* the per-update `Probe` boundary, and it
                // reports latency, not throughput. `aggregate` suppresses
                // throughput for this pass because the mask lacks the bit.
                let sink = FullSink::new(pass_cfg.metrics);
                run_once(factory, sink, items, &pass_cfg)
            } else {
                run_once_clean(factory, insert, items, &pass_cfg)
            };

            if run_idx >= pass_cfg.warmup_runs {
                let mut metrics = metrics;
                if pass_cfg.metrics.contains(MetricsMask::ACCURACY) {
                    if let Some(gt) = ground_truth {
                        let cmp = gt.compare(&final_sketch, items);
                        // Query phase ran inside the comparator; pull
                        // its timing into the run's metrics so
                        // aggregation surfaces query_throughput
                        // alongside insertion throughput.
                        metrics.queries_executed = cmp.queries;
                        metrics.query_wall_time_ns = cmp.query_wall_ns;
                        metrics.accuracy = Some(cmp.json);
                        metrics.query_calls = cmp.query_calls;
                    }
                }
                metrics.memory_bytes = Some(final_sketch.memory_bytes() as u64);
                per_run.push(metrics);
            }
        }

        let bench = aggregate(&per_run, pass_cfg.metrics);
        BenchReport {
            sketch: self.sketch_name.clone(),
            impl_name: self.impl_name.clone(),
            workload: self.workload.desc(),
            per_run,
            bench,
            config: pass_cfg,
        }
    }
}

/// **The** insert loop. Times `insert` over every item and returns the
/// elapsed nanoseconds.
///
/// Every pass that reports throughput goes through here, and nothing else
/// is inside the timed region — no metric snapshot, no `finalize_for_query`,
/// no `memory_bytes`. This function existing exactly once is a correctness
/// property, not tidiness:
///
/// The throughput fast path and the CPU/MEMORY path used to carry their own
/// copies of this loop, one calling a closure supplied by `sketch-cli` and
/// the other calling `sketch.update(it)` from inside `sketch-bench`. Both
/// timed the right region, so the bug was invisible to review — but the
/// cross-crate call in the second copy cost the asap_sketchlib FixedMatrix
/// FastPath its inlining, and `cms/lib-fixedmatrix-fast-32k` reported
/// **5.1% lower throughput** under `--metrics throughput,cpu,memory` than
/// under `--metrics throughput` (5 alternating rounds, non-overlapping
/// ranges). The penalty scaled with how much an implementation relies on
/// inlining — ~1.5% for `hll/oxide` — so it did not cancel out: it changed
/// the ranking *between* implementations, and the slow path is the one the
/// default `--metrics` selects.
#[inline(always)]
fn insert_loop<S, Insert>(sketch: &mut S, items: &[S::Item], insert: &mut Insert) -> u64
where
    S: Sketch,
    Insert: FnMut(&mut S, &S::Item),
{
    let start = Instant::now();
    for it in items {
        insert(sketch, it);
    }
    let ns = start.elapsed().as_nanos() as u64;
    std::hint::black_box(&*sketch);
    ns
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

/// One measured run with no per-update instrumentation. Used by
/// the THROUGHPUT and ACCURACY passes — both want a clean insert
/// hot path with no `Probe<S, FullSink>` wrapper, so the inner
/// sketch's `update` is the only thing in the loop. Phase-boundary
/// metrics (CPU / MEMORY / heap-track) still attach via direct
/// primitives instead of going through `FullSink`.
#[inline(always)]
fn run_once_clean<S, F, Insert>(
    factory: &mut F,
    insert: &mut Insert,
    items: &[S::Item],
    config: &BenchConfig,
) -> (RunMetrics, S)
where
    S: Sketch,
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
    sketch.finalize_for_query();
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

/// Placeholder `GroundTruth` for dispatch rows that run without a
/// comparator. Never called; `compare` is a safe default.
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
