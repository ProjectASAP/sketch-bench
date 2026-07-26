//! `MetricsMask` — which metric families a sink collects.
//!
//! Named by both consumers of the `MetricsSink` contract — the offline
//! [`FullSink`](crate::metrics::FullSink) and the embedded
//! `sketch-runtime::Sampler` — which is why it sits in `aqpbm-core` rather
//! than in either of them.
//!
//! See `docs/DESIGN.md` §5.3.

use bitflags::bitflags;

bitflags! {
    /// Which metric families are collected during a run. Each bit gates both
    /// the construction cost and the hot-path overhead of its recorder:
    /// `FullSink` holds only the recorders whose bits are set. An empty mask
    /// is legal, and useful as a minimal smoke test.
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

/// Every primary pass, paired with the name that identifies it in a record.
const PRIMARY_PASSES: [(MetricsMask, &str); 4] = [
    (MetricsMask::THROUGHPUT, "throughput"),
    (MetricsMask::LATENCY, "latency"),
    (MetricsMask::ACCURACY, "accuracy"),
    (MetricsMask::MERGE, "merge"),
];

impl MetricsMask {
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
        for (primary, _) in PRIMARY_PASSES {
            if self.contains(primary) {
                out.push(primary | secondary);
            }
        }
        if out.is_empty() && !secondary.is_empty() {
            out.push(secondary);
        }
        out
    }

    /// PASS bit to str name
    pub fn pass_name(self) -> Option<&'static str> {
        PRIMARY_PASSES
            .iter()
            .find(|(bit, _)| self.contains(*bit))
            .map(|(_, name)| *name)
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

    #[test]
    fn every_pass_has_a_distinct_name() {
        // The property consumers rely on: a record's `pass` identifies which
        // run produced it, so two passes must never share a name.
        let names: Vec<&str> = MetricsMask::all()
            .passes()
            .iter()
            .map(|p| p.pass_name().expect("every pass has a primary bit"))
            .collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "duplicate pass name in {names:?}"
        );
    }

    #[test]
    fn every_named_pass_runs_under_all() {
        // A bit added to the table must produce a pass, not just a name.
        assert_eq!(MetricsMask::all().passes().len(), PRIMARY_PASSES.len());
    }

    #[test]
    fn a_secondary_only_pass_has_no_name() {
        // CPU/MEMORY attach to a pass, they do not constitute one.
        let secondary = (MetricsMask::CPU | MetricsMask::MEMORY).passes();
        assert_eq!(secondary.len(), 1);
        assert_eq!(secondary[0].pass_name(), None);
    }
}
