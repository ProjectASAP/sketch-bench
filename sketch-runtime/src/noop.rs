//! Zero-cost fallback Sampler for `--no-default-features` builds: a zero-sized
//! newtype whose `MetricsSink` impl is four empty methods the compiler inlines
//! away. The knobs stay in the API signature so downstream code compiles
//! identically; they are simply ignored.

use std::time::Duration;

use aqpbm_core::probe::MetricsSink;
use aqpbm_core::report::Source;

use crate::exporter::Exporter;
use crate::switch::RuntimeSwitch;

#[derive(Debug, Clone)]
pub struct Tag {
    pub sketch: String,
    pub library: String,
    pub source: Source,
}

impl Tag {
    pub fn new(sketch: impl Into<String>, library: impl Into<String>, source: Source) -> Self {
        Self {
            sketch: sketch.into(),
            library: library.into(),
            source,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Disabled,
    EveryN {
        sample_every_n: u32,
        samples_per_window: u32,
    },
    EveryNTimeWindow {
        sample_every_n: u32,
        window: Duration,
    },
    EveryPeriod {
        sample_period: Duration,
        samples_per_window: u32,
    },
    EveryPeriodTimeWindow {
        sample_period: Duration,
        window: Duration,
    },
}

pub struct Sampler<E: Exporter> {
    _exporter: E,
    _tag: Tag,
}

impl<E: Exporter> Sampler<E> {
    pub fn disabled(exporter: E, tag: Tag) -> Self {
        Self {
            _exporter: exporter,
            _tag: tag,
        }
    }

    pub fn every_n(_: u32, _: u32, exporter: E, tag: Tag) -> Self {
        Self::disabled(exporter, tag)
    }

    pub fn every_n_time_window(_: u32, _: Duration, exporter: E, tag: Tag) -> Self {
        Self::disabled(exporter, tag)
    }

    pub fn every_period(_: Duration, _: u32, exporter: E, tag: Tag) -> Self {
        Self::disabled(exporter, tag)
    }

    pub fn every_period_time_window(_: Duration, _: Duration, exporter: E, tag: Tag) -> Self {
        Self::disabled(exporter, tag)
    }

    #[deprecated(note = "renamed to every_n_time_window")]
    pub fn time_window(_: u32, _: Duration, exporter: E, tag: Tag) -> Self {
        Self::disabled(exporter, tag)
    }

    pub fn with_switch(self, _: RuntimeSwitch) -> Self {
        self
    }

    pub fn without_latency(self) -> Self {
        self
    }

    pub fn switch(&self) -> RuntimeSwitch {
        RuntimeSwitch::off()
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        false
    }
}

impl<E: Exporter> MetricsSink for Sampler<E> {
    #[inline(always)]
    fn on_update_start(&mut self) {}
    #[inline(always)]
    fn on_update_end(&mut self) {}
    #[inline(always)]
    fn on_query_start(&mut self) {}
    #[inline(always)]
    fn on_query_end(&mut self) {}
}
