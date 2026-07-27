//! Per-update latency histogram, backed by `hdrhistogram` when the `hdrhist`
//! feature is on (the default). With it off, `LatencyRecorder` is a no-op shim
//! so the rest of the code stays agnostic to the optional dep. Lives here
//! because `sketch-runtime::Sampler` records latency on its hot path too.

#[cfg(feature = "hdrhist")]
use hdrhistogram::Histogram;

/// Quantiles read off a [`LatencyRecorder`] at finalize time.
/// Feeds `report::LatencySummary` on its way into the v1 record.
#[derive(Debug, Clone, Default)]
pub struct LatencySnapshot {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
    pub count: u64,
}

pub struct LatencyRecorder {
    #[cfg(feature = "hdrhist")]
    hist: Histogram<u64>,
    #[cfg(feature = "hdrhist")]
    start_ns: Option<std::time::Instant>,
    #[cfg(not(feature = "hdrhist"))]
    count: u64,
}

impl LatencyRecorder {
    pub fn new() -> Self {
        #[cfg(feature = "hdrhist")]
        {
            // 1 ns → 10 s range, 3 significant figures. Covers
            // every sketch-update latency we care about.
            let hist = Histogram::<u64>::new_with_bounds(1, 10_000_000_000, 3)
                .expect("valid histogram bounds");
            Self {
                hist,
                start_ns: None,
            }
        }
        #[cfg(not(feature = "hdrhist"))]
        {
            Self { count: 0 }
        }
    }

    #[inline]
    pub fn on_start(&mut self) {
        #[cfg(feature = "hdrhist")]
        {
            self.start_ns = Some(std::time::Instant::now());
        }
    }

    #[inline]
    pub fn on_end(&mut self) {
        #[cfg(feature = "hdrhist")]
        {
            if let Some(s) = self.start_ns.take() {
                let dur = s.elapsed().as_nanos() as u64;
                // Clamp to histogram range — outliers get stuffed into
                // the top bucket rather than failing the record.
                let _ = self.hist.record(dur.max(1));
            }
        }
        #[cfg(not(feature = "hdrhist"))]
        {
            self.count = self.count.saturating_add(1);
        }
    }

    /// Record a pre-measured duration in nanoseconds. Used by
    /// `sketch-runtime::Sampler` which measures the sampled
    /// op's wall-time outside the recorder.
    #[inline]
    pub fn record_ns(&mut self, ns: u64) {
        #[cfg(feature = "hdrhist")]
        {
            let _ = self.hist.record(ns.max(1));
        }
        #[cfg(not(feature = "hdrhist"))]
        {
            let _ = ns;
            self.count = self.count.saturating_add(1);
        }
    }

    pub fn snapshot(&self) -> LatencySnapshot {
        #[cfg(feature = "hdrhist")]
        {
            LatencySnapshot {
                p50: self.hist.value_at_quantile(0.50),
                p95: self.hist.value_at_quantile(0.95),
                p99: self.hist.value_at_quantile(0.99),
                p999: self.hist.value_at_quantile(0.999),
                max: self.hist.max(),
                count: self.hist.len(),
            }
        }
        #[cfg(not(feature = "hdrhist"))]
        {
            LatencySnapshot {
                count: self.count,
                ..Default::default()
            }
        }
    }
}

impl Default for LatencyRecorder {
    fn default() -> Self {
        Self::new()
    }
}
