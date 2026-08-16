//! Timing a closure. That is all this module does, and all core knows how to do.
//!
//! A caller hands [`measure`] a body. The body builds whatever it needs, marks
//! the region it wants timed with [`Timed::time`], and reports what it did.
//! Core runs it `warmup_runs + runs` times, wraps the recorders around the
//! marked region, and folds the results.
//!
//! Nothing here knows what a sketch is, which operations exist, or that
//! "insert" and "query" are different words. The caller decides what to build,
//! what to time, and what to call it — see `sketch_bench::registry`.

use std::collections::BTreeMap;
use std::sync::Once;
use std::time::{Duration, Instant};

use crate::latency::LatencyRecorder;
use crate::metrics::memory::{JemallocAllocated, Rss};
use crate::metrics::time::{CpuTimeSampler, WallClock};
use crate::metrics::{MetricsMask, RunMetrics};

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

/// What one execution of a body did.
///
/// The body reports this because only the body can know it: core sees an opaque
/// closure, so it cannot count items or size a structure. `memory_bytes` in
/// particular has to be read here — a body that owns its sketch drops it on the
/// way out, and a heap delta measured afterwards would read zero.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    /// How many units of work the timed region covered: items inserted, probes
    /// asked, shards folded. The numerator of any rate.
    pub work: u64,
    /// The nominal footprint the body claims, computed before it drops anything.
    pub memory_bytes: Option<u64>,
    /// Named scalars — error metrics, probe counts. Empty for a pure timing.
    pub scores: BTreeMap<String, f64>,
}

/// The clock, handed to the body so it can say where the timed region is.
///
/// Setup happens outside it: a body rebuilds its sketch every run, and a query
/// body refills one, but neither belongs in the number. Marking the region is
/// the body's job because only the body knows which part is the measurement.
pub struct Timed {
    elapsed_ns: u64,
    latency: Option<LatencyRecorder>,
}

impl Timed {
    fn new(mask: MetricsMask) -> Self {
        Self {
            elapsed_ns: 0,
            latency: mask
                .contains(MetricsMask::LATENCY)
                .then(LatencyRecorder::new),
        }
    }

    /// Time `f` as one region. One clock read either side, nothing inside.
    #[inline(always)]
    pub fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let clock = WallClock::start();
        let out = f();
        self.elapsed_ns += clock.elapsed_ns();
        out
    }

    /// Time `f` per element, feeding each duration to the latency recorder.
    ///
    /// Reading the clock inside the loop is what a latency distribution needs
    /// and what a throughput number must not pay for, so a body picks one.
    #[inline(always)]
    pub fn time_each<X, T>(&mut self, xs: &[X], mut f: impl FnMut(&X) -> T) {
        let clock = WallClock::start();
        match self.latency.as_mut() {
            Some(rec) => {
                for x in xs {
                    let t0 = WallClock::start();
                    std::hint::black_box(f(x));
                    rec.record_ns(t0.elapsed_ns());
                }
            }
            None => {
                for x in xs {
                    std::hint::black_box(f(x));
                }
            }
        }
        self.elapsed_ns += clock.elapsed_ns();
    }
}

/// Run `body` `warmup_runs + runs` times; return the measured runs' metrics.
///
/// Warm-ups are run and discarded, so a body that rebuilds per run pays its
/// setup on those too — which is the point of them.
pub fn measure<F>(cfg: &MeasureConfig, mut body: F) -> Vec<RunMetrics>
where
    F: FnMut(&mut Timed) -> RunOutcome,
{
    warmup_cpu_once();
    let mut out = Vec::with_capacity(cfg.runs);
    for i in 0..(cfg.warmup_runs + cfg.runs) {
        let mut cpu = cfg
            .metrics
            .contains(MetricsMask::CPU)
            .then(CpuTimeSampler::start);

        #[cfg(feature = "heap-track")]
        let heap_before = {
            crate::metrics::heap_track::reset_peak();
            crate::metrics::heap_track::snapshot()
        };

        let mut timed = Timed::new(cfg.metrics);
        let outcome = body(&mut timed);

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
            elapsed_ns: timed.elapsed_ns,
            cpu_user_ns,
            cpu_sys_ns,
            rss_peak_kb,
            heap_allocated_kb,
            memory_bytes: outcome.memory_bytes,
            heap_bytes_net: None,
            heap_bytes_peak: None,
            latency_ns: timed.latency.map(|r| r.snapshot()),
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

/// Ramp the CPU **once per process**, before the first measured region of the
/// first measurement — two measurements timed at two different clock states are
/// not a comparison. `Once` is what keeps the second measurement in a process
/// from re-burning it.
///
/// Duration comes from `BENCH_WARMUP_SECS` and **defaults to 0**: a library
/// must not burn a caller's CPU uninvited. `approxbench`'s `main` sets the
/// measurement default, so the number a benchmark reports is a ramped one.
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
