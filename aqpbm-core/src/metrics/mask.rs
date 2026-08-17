//! `MetricsMask` — which recorders a measurement arms — and `OperationMask`,
//! what it arms them over. Neither belongs to a recorder: they select across all
//! of them, so they sit beside the recorders rather than inside one.
//!
//! Both are *vocabulary*, not requests. A registry entry declares what it admits
//! with them, and `MeasureConfig` arms recorders with them. What a frontend asks
//! for is one [`Operation`] and one [`Metric`] — see [`crate::ops::Target::body`].
//!
//! See `docs/aqpbm-core.md` §Metrics and §Operations.

use bitflags::bitflags;

bitflags! {
    /// Which metric algorithms are collected during a run. Each bit gates both the
    /// construction cost and the hot-path overhead of its recorder. An empty mask
    /// is legal, and useful as a minimal smoke test.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MetricsMask: u32 {
        const THROUGHPUT = 1 << 0;
        const LATENCY    = 1 << 1;
        const CPU        = 1 << 2;
        const MEMORY     = 1 << 3;
        const ACCURACY   = 1 << 4;
    }
}

bitflags! {
    /// Which operations a target admits. A separate set from [`MetricsMask`]:
    /// one says *what is measured*, this says *what it is measured over*.
    /// Insert and query are assumed; merge and prepare are asked for by name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct OperationMask: u32 {
        const INSERT  = 1 << 0;
        const QUERY   = 1 << 1;
        const MERGE   = 1 << 2;
        const PREPARE = 1 << 3;
    }
}

/// One operation, as a value. The mask is a set and cannot be matched
/// exhaustively; this can, which is what lets the runner's dispatch be
/// checked by the compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Insert,
    Query,
    Merge,
    Prepare,
}

/// One metric, as a value. Same reason as [`Operation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Throughput,
    Latency,
    Accuracy,
}

impl Operation {
    /// Every operation, in the order a frontend enumerating them should use.
    pub const ALL: [Operation; 4] = [
        Operation::Insert,
        Operation::Query,
        Operation::Merge,
        Operation::Prepare,
    ];

    /// The mask bit standing for this operation. The bit and the value are two
    /// spellings of one thing, and this is the only place that says so.
    pub fn bit(self) -> OperationMask {
        match self {
            Operation::Insert => OperationMask::INSERT,
            Operation::Query => OperationMask::QUERY,
            Operation::Merge => OperationMask::MERGE,
            Operation::Prepare => OperationMask::PREPARE,
        }
    }

    /// The name this operation carries in a record.
    pub fn name(self) -> &'static str {
        match self {
            Operation::Insert => "insert",
            Operation::Query => "query",
            Operation::Merge => "merge",
            Operation::Prepare => "prepare",
        }
    }
}

impl Metric {
    /// Every metric that names a measurement of its own, in enumeration order.
    /// Cpu and memory are absent on purpose: they are [`MetricsMask::SECONDARY`],
    /// and ride along with a measurement rather than being one.
    pub const ALL: [Metric; 3] = [Metric::Throughput, Metric::Latency, Metric::Accuracy];

    /// The mask bit standing for this metric. Also what a caller ors with
    /// [`MetricsMask::SECONDARY`] to arm exactly one measurement's recorders.
    pub fn bit(self) -> MetricsMask {
        match self {
            Metric::Throughput => MetricsMask::THROUGHPUT,
            Metric::Latency => MetricsMask::LATENCY,
            Metric::Accuracy => MetricsMask::ACCURACY,
        }
    }

    /// The name this metric carries in a record's `metric` field.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Throughput => "throughput",
            Metric::Latency => "latency",
            Metric::Accuracy => "accuracy",
        }
    }
}

/// Whether the framework measures this pair *at all*, for any target. A fact about
/// the metrics, not about a request: a caller asks before it asks a registry
/// whether some particular entry has it.
pub fn is_measurable(operation: Operation, metric: Metric) -> bool {
    use Metric::{Accuracy, Latency, Throughput};
    use Operation::{Insert, Merge, Prepare, Query};
    match (operation, metric) {
        (Insert, Throughput) | (Insert, Latency) => true,
        (Insert, Accuracy) => false,
        (Query, Throughput) | (Query, Accuracy) => true,
        (Query, Latency) => false,
        (Merge, Throughput) | (Merge, Latency) => true,
        (Merge, Accuracy) => false,
        (Prepare, Throughput) => false,
        (Prepare, Latency) => true,
        (Prepare, Accuracy) => false,
    }
}

impl MetricsMask {
    /// Bits that record at phase boundaries only (start / finish of the insert
    /// phase, not per-update). They contaminate nothing, so they ride along
    /// with every measurement instead of naming one of their own.
    pub const SECONDARY: MetricsMask =
        MetricsMask::from_bits_truncate(Self::CPU.bits() | Self::MEMORY.bits());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_and_its_bit_are_two_spellings_of_one_thing() {
        for op in Operation::ALL {
            assert!(OperationMask::all().contains(op.bit()), "{}", op.name());
        }
        for metric in Metric::ALL {
            assert!(
                MetricsMask::all().contains(metric.bit()),
                "{}",
                metric.name()
            );
        }
    }

    #[test]
    fn secondary_names_no_measurement_of_its_own() {
        // Cpu and memory attach to a measurement; they are not one, so no
        // `Metric` value maps onto either bit.
        for metric in Metric::ALL {
            assert!(
                !MetricsMask::SECONDARY.contains(metric.bit()),
                "{}",
                metric.name()
            );
        }
    }

    #[test]
    fn five_of_the_twelve_pairs_are_permanently_empty() {
        // Four operations by three metrics is the whole of what this crate can
        // be asked for, whatever it happens to implement.
        let pairs: Vec<_> = Operation::ALL
            .into_iter()
            .flat_map(|op| Metric::ALL.map(|m| (op, m)))
            .collect();
        assert_eq!(pairs.len(), 12);

        let empty: Vec<_> = pairs
            .into_iter()
            .filter(|(op, m)| !is_measurable(*op, *m))
            .map(|(op, m)| (op.name(), m.name()))
            .collect();
        assert_eq!(
            empty,
            vec![
                ("insert", "accuracy"),
                ("query", "latency"),
                ("merge", "accuracy"),
                ("prepare", "throughput"),
                ("prepare", "accuracy"),
            ]
        );
    }
}
