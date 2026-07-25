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

/// Every primary pass, paired with the name that identifies it in a record.
const PRIMARY_PASSES: [(MetricsMask, &str); 4] = [
    (MetricsMask::THROUGHPUT, "throughput"),
    (MetricsMask::LATENCY, "latency"),
    (MetricsMask::ACCURACY, "accuracy"),
    (MetricsMask::MERGE, "merge"),
];

impl MetricsMask {
    /// Bits that each need their own pass over a fresh sketch, because they
    /// share the per-`update` hot path and would contaminate each other:
    /// bracketing every update with two `Instant::now()` calls for LATENCY
    /// inflates the THROUGHPUT denominator by exactly that overhead.
    ///
    /// A property of the metrics, not of the offline runner — an embedded
    /// sampler enabling both on one path would skew the same way.
    pub const PRIMARY: MetricsMask = {
        // Folded from the table rather than re-listed, so the two cannot
        // disagree about what "primary" means.
        let mut bits = 0u32;
        let mut i = 0;
        while i < PRIMARY_PASSES.len() {
            bits |= PRIMARY_PASSES[i].0.bits();
            i += 1;
        }
        MetricsMask::from_bits_truncate(bits)
    };

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
    fn primary_is_exactly_the_named_passes() {
        // `PRIMARY` is folded from the same table `pass_name` reads, so this
        // cannot drift — it pins that the fold is what we think it is, and
        // that a bit added to the table lands in `PRIMARY` for free.
        assert_eq!(
            MetricsMask::PRIMARY,
            MetricsMask::THROUGHPUT
                | MetricsMask::LATENCY
                | MetricsMask::ACCURACY
                | MetricsMask::MERGE
        );
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
