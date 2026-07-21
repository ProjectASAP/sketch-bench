//! `Sampler<E>` — the embedded `MetricsSink` that downstream
//! apps hand to a `Probe<S, Sampler>`. Three modes:
//!
//! * **Disabled** — zero work on the hot path (one cold branch).
//! * **EveryN { sample_every_n, samples_per_window }** — sample
//!   one op in every `sample_every_n`; emit a v1 JSONL record
//!   to the configured `Exporter` every `samples_per_window`
//!   sampled ops.
//! * **TimeWindow { sample_every_n, window }** — same sampling
//!   rate, but emit on a time boundary rather than count.
//!
//! See `docs/DESIGN.md` §7.1.

use std::time::{Duration, Instant};

use sketch_bench::metrics::LatencyRecorder;
use sketch_bench::MetricsMask;
use sketch_core::probe::MetricsSink;
use sketch_core::report::{
    BenchSection, LatencySummary, Mode as RecordMode, Record, RunStats, Source,
};
use sketch_core::workload::WorkloadDesc;

use crate::exporter::Exporter;
use crate::switch::RuntimeSwitch;

/// Tag material stamped on every emitted `Record`. Mirrors the
/// fields `BenchRunner` fills from its config so offline and
/// runtime records are shape-compatible.
#[derive(Debug, Clone)]
pub struct Tag {
    pub sketch: String,
    pub impl_name: String,
    pub source: Source,
}

impl Tag {
    pub fn new(sketch: impl Into<String>, impl_name: impl Into<String>, source: Source) -> Self {
        Self {
            sketch: sketch.into(),
            impl_name: impl_name.into(),
            source,
        }
    }
}

/// Sampling mode. Callers pick one at construction; the
/// `Disabled` variant is what lets a single call site stay in
/// the source tree with zero runtime cost.
///
/// Two independent axes: *when to sample* (count vs. time) and
/// *when to emit* (count-of-samples vs. time since last emit).
/// Four constructors cover the four combinations — see
/// [`Sampler::every_n`], [`Sampler::every_n_time_window`],
/// [`Sampler::every_period`], [`Sampler::every_period_time_window`].
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    /// Never sample, never emit.
    Disabled,
    /// Sample 1 in `sample_every_n` ops; flush a record to the
    /// exporter every `samples_per_window` sampled ops.
    /// Suitable for sparse event-rate sampling
    /// (`sample_every_n = 1_000_000` → "once every million ops").
    EveryN {
        sample_every_n: u32,
        samples_per_window: u32,
    },
    /// Same 1-in-N event sampling; flush on a time boundary
    /// instead of a count boundary.
    EveryNTimeWindow {
        sample_every_n: u32,
        window: Duration,
    },
    /// Sample at most once per `sample_period` of wall time;
    /// flush every `samples_per_window` samples. Suitable for
    /// "every 1 s sample once" deployments where the app's op
    /// rate is unpredictable.
    EveryPeriod {
        sample_period: Duration,
        samples_per_window: u32,
    },
    /// Time-rate sampling, time-based emission window.
    EveryPeriodTimeWindow {
        sample_period: Duration,
        window: Duration,
    },
}

/// Hot-path state.
struct State {
    op_count: u64,
    sampled_ops: u32,
    window_started: Instant,
    /// Last wall-time a sample was actually measured. Used by
    /// the `EveryPeriod*` modes to rate-limit sampling on wall
    /// time regardless of op rate.
    last_sample_at: Option<Instant>,
    latency: Option<LatencyRecorder>,
    last_op_start: Option<Instant>,
}

impl State {
    fn new(mask: MetricsMask) -> Self {
        Self {
            op_count: 0,
            sampled_ops: 0,
            window_started: Instant::now(),
            last_sample_at: None,
            latency: if mask.contains(MetricsMask::LATENCY) {
                Some(LatencyRecorder::new())
            } else {
                None
            },
            last_op_start: None,
        }
    }
}

/// Embedded sampler + exporter. Implements `MetricsSink` so it
/// slots into any `Probe<S, Sampler<E>>`.
pub struct Sampler<E: Exporter> {
    mode: Mode,
    switch: RuntimeSwitch,
    exporter: E,
    tag: Tag,
    mask: MetricsMask,
    state: State,
}

impl<E: Exporter> Sampler<E> {
    /// Build a `Disabled` sampler — zero-op on the hot path.
    /// Useful as a no-op placeholder when a runtime knob decides
    /// at startup that no benchmarking is wanted.
    pub fn disabled(exporter: E, tag: Tag) -> Self {
        Self {
            mode: Mode::Disabled,
            switch: RuntimeSwitch::on(),
            exporter,
            tag,
            mask: MetricsMask::THROUGHPUT,
            state: State::new(MetricsMask::THROUGHPUT),
        }
    }

    /// Event-rate sampling, **count**-based emit window.
    ///
    /// * `sample_every_n` — sample 1 in every N ops. Set to 1 to
    ///   measure every op (the "scrape-every-op" batching mode
    ///   described in the crate README).
    /// * `samples_per_window` — emit a record after this many
    ///   sampled ops have accumulated.
    ///
    /// Total ops between emits ≈ `sample_every_n × samples_per_window`.
    pub fn every_n(sample_every_n: u32, samples_per_window: u32, exporter: E, tag: Tag) -> Self {
        Self::with_mode(
            Mode::EveryN {
                sample_every_n: sample_every_n.max(1),
                samples_per_window: samples_per_window.max(1),
            },
            exporter,
            tag,
        )
    }

    /// Event-rate sampling, **time**-based emit window.
    ///
    /// This is the "scrape every op, batch-emit once per second"
    /// pattern: `every_n_time_window(1, Duration::from_secs(1))`
    /// measures every op, accumulates the latency histogram +
    /// throughput counters in memory, then emits one batched
    /// record per wall-second — regardless of op rate.
    pub fn every_n_time_window(
        sample_every_n: u32,
        window: Duration,
        exporter: E,
        tag: Tag,
    ) -> Self {
        Self::with_mode(
            Mode::EveryNTimeWindow {
                sample_every_n: sample_every_n.max(1),
                window,
            },
            exporter,
            tag,
        )
    }

    /// Time-rate sampling, count-based emit window.
    ///
    /// Sample at most once per `sample_period` of wall time —
    /// independent of op rate. Suitable when the app's op rate
    /// is variable or unknown: "I want one measurement per
    /// second, no matter whether the host is doing 10 ops/s or
    /// 10 M ops/s."
    pub fn every_period(
        sample_period: Duration,
        samples_per_window: u32,
        exporter: E,
        tag: Tag,
    ) -> Self {
        Self::with_mode(
            Mode::EveryPeriod {
                sample_period,
                samples_per_window: samples_per_window.max(1),
            },
            exporter,
            tag,
        )
    }

    /// Time-rate sampling, time-based emit window.
    ///
    /// "Sample once per 10 ms, emit one batched record per 1 s"
    /// → `every_period_time_window(Duration::from_millis(10),
    /// Duration::from_secs(1))`.
    pub fn every_period_time_window(
        sample_period: Duration,
        window: Duration,
        exporter: E,
        tag: Tag,
    ) -> Self {
        Self::with_mode(
            Mode::EveryPeriodTimeWindow {
                sample_period,
                window,
            },
            exporter,
            tag,
        )
    }

    fn with_mode(mode: Mode, exporter: E, tag: Tag) -> Self {
        let mask = MetricsMask::THROUGHPUT | MetricsMask::LATENCY;
        Self {
            mode,
            switch: RuntimeSwitch::on(),
            exporter,
            tag,
            mask,
            state: State::new(mask),
        }
    }

    /// Back-compat alias for [`every_n_time_window`].
    #[deprecated(
        note = "renamed to every_n_time_window for symmetry with every_period_time_window"
    )]
    pub fn time_window(sample_every_n: u32, window: Duration, exporter: E, tag: Tag) -> Self {
        Self::every_n_time_window(sample_every_n, window, exporter, tag)
    }

    /// Attach an externally-owned toggle so the controller can
    /// flip sampling on/off without re-building the probe.
    pub fn with_switch(mut self, switch: RuntimeSwitch) -> Self {
        self.switch = switch;
        self
    }

    /// Opt-out of latency measurement. Drops the hdrhistogram +
    /// the `Instant::now()` on every sampled op.
    pub fn without_latency(mut self) -> Self {
        self.mask.remove(MetricsMask::LATENCY);
        self.state.latency = None;
        self
    }

    pub fn switch(&self) -> &RuntimeSwitch {
        &self.switch
    }

    /// Whether *both* the compile-time feature is on (implied by
    /// this being the `enabled` `Sampler`) AND the runtime
    /// switch says yes AND the mode isn't `Disabled`.
    #[inline]
    pub fn is_active(&self) -> bool {
        !matches!(self.mode, Mode::Disabled) && self.switch.is_enabled()
    }

    #[inline]
    fn should_sample(&self) -> bool {
        match self.mode {
            Mode::Disabled => false,
            Mode::EveryN { sample_every_n, .. } | Mode::EveryNTimeWindow { sample_every_n, .. } => {
                self.state.op_count.is_multiple_of(sample_every_n as u64)
            }
            Mode::EveryPeriod { sample_period, .. }
            | Mode::EveryPeriodTimeWindow { sample_period, .. } => {
                match self.state.last_sample_at {
                    None => true, // first op always seeds the clock
                    Some(last) => last.elapsed() >= sample_period,
                }
            }
        }
    }

    fn maybe_flush(&mut self) {
        let should_flush = match self.mode {
            Mode::Disabled => false,
            Mode::EveryN {
                samples_per_window, ..
            }
            | Mode::EveryPeriod {
                samples_per_window, ..
            } => self.state.sampled_ops >= samples_per_window,
            Mode::EveryNTimeWindow { window, .. } | Mode::EveryPeriodTimeWindow { window, .. } => {
                self.state.window_started.elapsed() >= window
            }
        };
        if should_flush {
            self.emit_window();
        }
    }

    fn emit_window(&mut self) {
        let wall_ns = self.state.window_started.elapsed().as_nanos() as u64;
        let throughput = if wall_ns == 0 {
            0.0
        } else {
            (self.state.op_count as f64) / (wall_ns as f64 / 1_000_000_000.0)
        };

        let latency = self.state.latency.as_ref().map(|rec| {
            let snap = rec.snapshot();
            LatencySummary {
                p50: snap.p50,
                p95: snap.p95,
                p99: snap.p99,
                p999: snap.p999,
                max: snap.max,
                count: snap.count,
            }
        });

        let bench = BenchSection {
            throughput_items_per_sec: Some(RunStats {
                mean: throughput,
                stddev: 0.0,
                ci95: None,
                n: 1,
            }),
            latency_ns: latency,
            wall_time_ms: Some(RunStats {
                mean: (wall_ns as f64) / 1_000_000.0,
                stddev: 0.0,
                ci95: None,
                n: 1,
            }),
            ..Default::default()
        };

        let wd = WorkloadDesc {
            shape: "live".into(),
            size: self.state.op_count as usize,
            cardinality: None,
            zipf_s: None,
            source_path: None,
            seed: None,
            // Live traffic has no generator spec to record.
            spec: None,
            dtype: Default::default(),
        };
        let mut rec = Record::new(
            self.tag.sketch.clone(),
            self.tag.impl_name.clone(),
            wd,
            RecordMode::Runtime,
            1,
        );
        rec.bench = Some(bench);
        rec.source = self.tag.source;
        self.exporter.export(&rec);

        // Reset window-scoped state.
        self.state = State::new(self.mask);
    }
}

impl<E: Exporter> MetricsSink for Sampler<E> {
    #[inline]
    fn on_update_start(&mut self) {
        if !self.is_active() {
            return;
        }
        if self.mask.contains(MetricsMask::LATENCY) && self.should_sample() {
            self.state.last_op_start = Some(Instant::now());
        }
    }

    #[inline]
    fn on_update_end(&mut self) {
        if !self.is_active() {
            return;
        }
        // Record op *before* incrementing so `should_sample`
        // uses the op-index that was active during the start
        // hook — keeps start/end paired for a single op.
        let sampled = self.should_sample();
        self.state.op_count = self.state.op_count.saturating_add(1);
        if sampled {
            if let (Some(rec), Some(start)) =
                (self.state.latency.as_mut(), self.state.last_op_start.take())
            {
                rec.record_ns(start.elapsed().as_nanos() as u64);
            }
            self.state.sampled_ops = self.state.sampled_ops.saturating_add(1);
            // `EveryPeriod*` modes need this to rate-limit the
            // next sample; `EveryN*` modes read op_count, which
            // is already advanced, and never look at this field
            // — so the unconditional stamp is cheap + harmless.
            self.state.last_sample_at = Some(Instant::now());
        }
        self.maybe_flush();
    }

    #[inline]
    fn on_query_start(&mut self) {}

    #[inline]
    fn on_query_end(&mut self) {}
}
