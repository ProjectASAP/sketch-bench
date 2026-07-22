//! `MetricsMask` — which metric families a sink collects.
//!
//! Lives here rather than in `sketch-bench` because both
//! consumers of the `MetricsSink` contract need it: the offline
//! `sketch-bench::FullSink` and the embedded
//! `sketch-runtime::Sampler`. Keeping it in `sketch-bench` meant
//! `sketch-runtime` had to depend on the whole offline benchmark
//! library — runner, baselines, accuracy comparators — to name
//! six bits.
//!
//! See `docs/DESIGN.md` §5.3.

use bitflags::bitflags;

bitflags! {
    /// Which metric families are collected during a run. Each
    /// bit gates both construction cost and hot-path overhead of
    /// its recorder — `Probe<_, FullSink>` only installs the
    /// recorders whose bits are set, and `FullSink` holds only
    /// those recorders. A mask with no bits set is legal (and
    /// useful as a minimal smoke-test).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MetricsMask: u32 {
        const THROUGHPUT = 1 << 0;
        const LATENCY    = 1 << 1;
        const CPU        = 1 << 2;
        const MEMORY     = 1 << 3;
        const ACCURACY   = 1 << 4;
        /// Build K shard sketches, time folding them into one, then compare
        /// the merged result against the whole stream.
        const MERGE      = 1 << 5;
    }
}

impl MetricsMask {
    /// Bits that share a hot path and therefore must be measured
    /// in separate passes to avoid mutual contamination. Each one
    /// gets its own `BenchRunner` pass with a fresh sketch.
    ///
    /// THROUGHPUT and LATENCY both gate the per-`update` boundary:
    /// throughput wants a clean hot path (just the inner sketch
    /// `update`); latency needs to bracket every `update` with two
    /// `Instant::now()` calls. Mixing them inflates the throughput
    /// denominator by exactly the latency-recorder overhead.
    ///
    /// ACCURACY also gets its own pass to keep its insert phase
    /// untainted, in case the user wants the accuracy comparator
    /// to see the same sketch state a clean-throughput run
    /// produces.
    ///
    /// This is a property of the metrics themselves, not of the
    /// offline runner: an embedded sampler that enabled both bits
    /// on one hot path would skew its throughput the same way.
    pub const PRIMARY: MetricsMask = MetricsMask::from_bits_truncate(
        Self::THROUGHPUT.bits() | Self::LATENCY.bits() | Self::ACCURACY.bits() | Self::MERGE.bits(),
    );

    /// Bits that record at phase boundaries only (start / finish
    /// of the insert phase, not per-update). Free to attach to
    /// any primary pass without contaminating its measurement.
    pub const SECONDARY: MetricsMask =
        MetricsMask::from_bits_truncate(Self::CPU.bits() | Self::MEMORY.bits());

    /// Split this mask into one sub-mask per `BenchRunner` pass.
    /// Each primary bit produces its own pass; secondary bits
    /// (CPU / MEMORY) attach to every primary pass. If no primary
    /// bit is set but secondary bits are, one pass runs with the
    /// secondary bits alone.
    pub fn passes(self) -> Vec<MetricsMask> {
        let secondary = self & Self::SECONDARY;
        let mut out = Vec::new();
        for primary in [Self::THROUGHPUT, Self::LATENCY, Self::ACCURACY, Self::MERGE] {
            if self.contains(primary) {
                out.push(primary | secondary);
            }
        }
        if out.is_empty() && !secondary.is_empty() {
            out.push(secondary);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_throughput_only() {
        let p = MetricsMask::THROUGHPUT.passes();
        assert_eq!(p, vec![MetricsMask::THROUGHPUT]);
    }

    #[test]
    fn passes_throughput_plus_latency_splits_into_two() {
        let p = (MetricsMask::THROUGHPUT | MetricsMask::LATENCY).passes();
        assert_eq!(p, vec![MetricsMask::THROUGHPUT, MetricsMask::LATENCY],);
    }

    #[test]
    fn passes_attaches_memory_cpu_to_every_primary() {
        let p = MetricsMask::all().passes();
        assert_eq!(p.len(), 4);
        for m in &p {
            assert!(m.contains(MetricsMask::CPU));
            assert!(m.contains(MetricsMask::MEMORY));
        }
    }

    #[test]
    fn passes_secondary_only_runs_one_pass() {
        let p = (MetricsMask::CPU | MetricsMask::MEMORY).passes();
        assert_eq!(p, vec![MetricsMask::CPU | MetricsMask::MEMORY]);
    }

    #[test]
    fn passes_empty_returns_empty() {
        assert!(MetricsMask::empty().passes().is_empty());
    }
}
