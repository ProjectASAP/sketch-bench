//! Timing a closure. That is all this module does, and all core knows how to do.
//! A caller hands over one primed closure per run; `measure` calls each with the
//! clock around it and folds what they report.

use std::collections::BTreeMap;
use std::sync::Once;
use std::time::{Duration, Instant};

use crate::metrics::latency::{LatencyRecorder, LatencySnapshot};
use crate::metrics::memory::{JemallocAllocated, Rss};
use crate::metrics::time::{CpuTimeSampler, WallClock};
use crate::metrics::{Metric, MetricsMask, RunMetrics};

/// How many times to run a body, and what to record around it.
///
/// The loop knobs, and nothing else. What is being measured, over which
/// operation, at which config — none of that reaches here.
#[derive(Debug, Clone)]
pub struct MeasureConfig {
    /// Measured iterations. The aggregate's population.
    pub runs: usize,
    /// Iterations run first and thrown away, so the first measured one is not
    /// paying for a cold cache or a lazy allocator.
    pub warmup_runs: usize,
    /// Which recorders to arm. An empty mask still runs the body.
    pub metrics: MetricsMask,
}

/// One run of a measurement, primed: whatever it needed built is already built,
/// so calling it *is* the work being timed. `FnOnce` because a filled sketch is
/// not a fresh one — a second run is a second closure, not a second call.
pub type Pass = Box<dyn FnOnce() -> Report>;

/// What the pass produced, read once the clock has stopped: the footprint of a
/// sketch the pass still owns, and whatever its answers scored. Separate from
/// [`Pass`] so neither reading nor scoring lands inside the measurement.
pub type Report = Box<dyn FnOnce() -> RunOutcome>;

/// One measurement: one primed pass per run, warm-ups included, in the order
/// they will be run.
pub type Measurement = Vec<Pass>;

/// The smallest fold that is a merge at all: two shards, one merge call. The
/// count itself comes from the request, and this only keeps a `--merge-shards 1`
/// from measuring an empty loop.
pub const MIN_MERGE_SHARDS: usize = 2;

/// How many measured runs a metric takes. Error is deterministic given (data,
/// parameters) and a dataset is drawn once, so repeating an accuracy
/// measurement would fabricate spread: ten identical answers averaged to
/// `stddev: 0.0` over `n: 10`. Timing is where repeating one draw *is* a
/// repeat. One rule, read by whoever builds the passes and by whoever
/// configures the loop.
pub fn runs_for(metric: Metric, runs: usize) -> usize {
    match metric {
        Metric::Accuracy => 1,
        _ => runs,
    }
}

/// Drive `steps` calls, timing each on its own. Reading the clock inside the
/// loop is what a latency distribution needs and what a throughput number must
/// not pay for, so a caller asks for this only when the metric does.
#[inline(always)]
pub fn record_calls(steps: usize, mut f: impl FnMut(usize)) -> LatencySnapshot {
    let mut rec = LatencyRecorder::new();
    for i in 0..steps {
        let clock = WallClock::start();
        f(i);
        rec.record_ns(clock.elapsed_ns());
    }
    rec.snapshot()
}

/// What one execution of a body did. The body reports it because core sees an
/// opaque closure: `memory_bytes` in particular has to be read here, since a
/// body that owns its sketch drops it on the way out.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    /// How many units of work the timed region covered: items inserted, probes
    /// asked, shards folded. The numerator of any rate.
    pub work: u64,
    /// The nominal footprint the body claims, computed before it drops anything.
    pub memory_bytes: Option<u64>,
    /// Named scalars — error metrics, probe counts. Empty for a pure timing.
    pub scores: BTreeMap<String, f64>,
    /// The per-call distribution, for the passes that timed themselves per
    /// item. `None` for a pass timed as one region.
    pub latency_ns: Option<LatencySnapshot>,
}

/// Run each pass with the clock around it; return the measured runs' metrics.
///
/// The first `warmup_runs` passes are run and discarded, so the setup they
/// paid for — a sketch built, a stream fed — is paid on those too, which is
/// the point of them.
pub fn measure(cfg: &MeasureConfig, passes: Measurement) -> Vec<RunMetrics> {
    warmup_cpu_once();
    let mut out = Vec::with_capacity(cfg.runs);
    for (i, pass) in passes.into_iter().enumerate() {
        let mut cpu = cfg
            .metrics
            .contains(MetricsMask::CPU)
            .then(CpuTimeSampler::start);

        #[cfg(feature = "heap-track")]
        let heap_before = {
            crate::metrics::heap_track::reset_peak();
            crate::metrics::heap_track::snapshot()
        };

        let clock = WallClock::start();
        let report = pass();
        let elapsed_ns = clock.elapsed_ns();
        let outcome = report();

        #[cfg(feature = "heap-track")]
        let heap_after = crate::metrics::heap_track::snapshot();

        if i < cfg.warmup_runs {
            continue;
        }

        let (cpu_user_ns, cpu_sys_ns) = match cpu.take() {
            Some(s) => {
                let s = s.finish();
                (Some(s.user_ns), Some(s.sys_ns))
            }
            None => (None, None),
        };
        let (rss_peak_kb, heap_allocated_kb) = if cfg.metrics.contains(MetricsMask::MEMORY) {
            (Rss::peak_kb(), JemallocAllocated::read_kb())
        } else {
            (None, None)
        };

        #[allow(unused_mut)]
        let mut m = RunMetrics {
            work: outcome.work,
            elapsed_ns,
            cpu_user_ns,
            cpu_sys_ns,
            rss_peak_kb,
            heap_allocated_kb,
            memory_bytes: outcome.memory_bytes,
            heap_bytes_net: None,
            heap_bytes_peak: None,
            latency_ns: outcome.latency_ns,
            scores: (!outcome.scores.is_empty()).then_some(outcome.scores),
        };

        #[cfg(feature = "heap-track")]
        {
            m.heap_bytes_net = Some((heap_after.in_use - heap_before.in_use).max(0) as u64);
            m.heap_bytes_peak = Some((heap_after.peak - heap_before.in_use).max(0) as u64);
        }

        out.push(m);
    }
    out
}

/// Ramp the CPU **once per process**, before the first measured region: two
/// measurements timed at different clock states are not a comparison. Duration
/// from `BENCH_WARMUP_SECS`, default 0 — a library must not burn CPU uninvited.
fn warmup_cpu_once() {
    static WARMED: Once = Once::new();
    WARMED.call_once(|| {
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
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
            }
            std::hint::black_box(x);
        }
    });
}
