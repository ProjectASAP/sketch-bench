//! `MetricsMask` — which metric algorithms a sink collects. Named by both
//! consumers of the `MetricsSink` contract, the offline
//! [`FullSink`](crate::metrics::FullSink) and `sketch-runtime::Sampler`, which
//! is why it sits here rather than in either. See `docs/DESIGN.md` §5.3.

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
    /// Which operations to measure over. A separate set from [`MetricsMask`]:
    /// one says *what is measured*, this says *what it is measured over*, and a
    /// request is the cross product of the two.
    ///
    /// Insert and query are assumed of every implementation; merge and prepare
    /// are declared, so a caller asks for them by name.
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

/// One measurement: a metric taken over an operation. This is what a record
/// names, and what the runner dispatches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub operation: Operation,
    pub metric: Metric,
    /// The phase-boundary bits that ride along. They contaminate nothing, so
    /// they attach to every cell instead of forming cells of their own.
    pub secondary: MetricsMask,
}

impl Cell {
    /// The bits this cell records: its own metric, plus the phase-boundary
    /// bits riding along. What the recorders and the aggregator gate on.
    pub fn mask(self) -> MetricsMask {
        let primary = match self.metric {
            Metric::Throughput => MetricsMask::THROUGHPUT,
            Metric::Latency => MetricsMask::LATENCY,
            Metric::Accuracy => MetricsMask::ACCURACY,
        };
        primary | self.secondary
    }
}

impl Operation {
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
    /// The name this metric carries in a record's `metric` field.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Throughput => "throughput",
            Metric::Latency => "latency",
            Metric::Accuracy => "accuracy",
        }
    }
}

/// Every operation the mask can hold, in the order cells are produced.
const OPERATIONS: [(OperationMask, Operation); 4] = [
    (OperationMask::INSERT, Operation::Insert),
    (OperationMask::QUERY, Operation::Query),
    (OperationMask::MERGE, Operation::Merge),
    (OperationMask::PREPARE, Operation::Prepare),
];

/// Every metric the mask can hold, in the order cells are produced.
const METRICS: [(MetricsMask, Metric); 3] = [
    (MetricsMask::THROUGHPUT, Metric::Throughput),
    (MetricsMask::LATENCY, Metric::Latency),
    (MetricsMask::ACCURACY, Metric::Accuracy),
];

/// Whether the framework measures this square *at all*, for any row.
///
/// Five of the twelve are permanently empty, and for reasons that hold of every
/// implementation rather than of any one of them: an insert produces no answer
/// to score, a fold produces no answer to score, scoring a folded sketch is the
/// query operation wearing merge's name, `prepare` runs at the tail of the
/// insert loop so it has a latency but no throughput of its own, and a query
/// latency needs each estimate call timed separately — a per-call capture this
/// build no longer has.
///
/// This mirrors the per-operation match in [`crate::ops::squares_for`], which is
/// the authority — it is the one that actually builds something to run. Stated
/// separately here so a *frontend* can refuse a square by name before
/// generating a dataset for it, which is the whole point of refusing early.
pub fn is_measurable(cell: Cell) -> bool {
    use Metric::{Accuracy, Latency, Throughput};
    use Operation::{Insert, Merge, Prepare, Query};
    match (cell.operation, cell.metric) {
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

/// The cells a request selects: every (operation, metric) the two masks name
/// between them. A cell with no implementation is still produced, and the
/// runner is what finds nothing to run for it.
pub fn cells(operations: OperationMask, metrics: MetricsMask) -> Vec<Cell> {
    let secondary = metrics & MetricsMask::SECONDARY;
    let mut out = Vec::new();
    for (op_bit, operation) in OPERATIONS {
        if !operations.contains(op_bit) {
            continue;
        }
        for (metric_bit, metric) in METRICS {
            if metrics.contains(metric_bit) {
                out.push(Cell {
                    operation,
                    metric,
                    secondary,
                });
            }
        }
    }
    out
}

impl MetricsMask {
    /// Bits that record at phase boundaries only (start / finish of the insert
    /// phase, not per-update). They contaminate nothing, so they ride along
    /// with every cell instead of forming cells of their own.
    pub const SECONDARY: MetricsMask =
        MetricsMask::from_bits_truncate(Self::CPU.bits() | Self::MEMORY.bits());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(cells: &[Cell]) -> Vec<(&'static str, &'static str)> {
        cells
            .iter()
            .map(|c| (c.operation.name(), c.metric.name()))
            .collect()
    }

    #[test]
    fn one_operation_one_metric_is_one_cell() {
        let c = cells(OperationMask::INSERT, MetricsMask::THROUGHPUT);
        assert_eq!(names(&c), vec![("insert", "throughput")]);
    }

    #[test]
    fn the_request_is_the_cross_product() {
        let c = cells(
            OperationMask::INSERT | OperationMask::MERGE,
            MetricsMask::THROUGHPUT | MetricsMask::LATENCY,
        );
        assert_eq!(
            names(&c),
            vec![
                ("insert", "throughput"),
                ("insert", "latency"),
                ("merge", "throughput"),
                ("merge", "latency"),
            ]
        );
    }

    #[test]
    fn a_cell_with_no_implementation_is_still_produced() {
        // Selection does not judge. Nothing implements insert accuracy; the
        // runner is what finds nothing to run for it.
        let c = cells(OperationMask::INSERT, MetricsMask::ACCURACY);
        assert_eq!(names(&c), vec![("insert", "accuracy")]);
    }

    #[test]
    fn secondary_bits_ride_along_and_form_no_cell_of_their_own() {
        let c = cells(
            OperationMask::INSERT,
            MetricsMask::THROUGHPUT | MetricsMask::CPU | MetricsMask::MEMORY,
        );
        assert_eq!(names(&c), vec![("insert", "throughput")]);
        assert_eq!(c[0].secondary, MetricsMask::CPU | MetricsMask::MEMORY);
    }

    #[test]
    fn secondary_bits_alone_select_nothing() {
        // They attach to a measurement; they are not one.
        assert!(cells(OperationMask::all(), MetricsMask::CPU | MetricsMask::MEMORY).is_empty());
    }

    #[test]
    fn an_empty_mask_on_either_axis_selects_nothing() {
        assert!(cells(OperationMask::empty(), MetricsMask::all()).is_empty());
        assert!(cells(OperationMask::all(), MetricsMask::empty()).is_empty());
    }

    #[test]
    fn five_of_the_twelve_squares_are_permanently_empty() {
        let empty: Vec<_> = cells(OperationMask::all(), MetricsMask::all())
            .into_iter()
            .filter(|c| !is_measurable(*c))
            .map(|c| (c.operation.name(), c.metric.name()))
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

    #[test]
    fn all_by_all_is_the_whole_grid() {
        // Four operations by three metrics. The grid is the thing this crate
        // can be asked for, whatever it happens to implement.
        assert_eq!(cells(OperationMask::all(), MetricsMask::all()).len(), 12);
    }

    #[test]
    fn a_cell_records_its_own_metric_plus_whatever_rides_along() {
        let c = cells(
            OperationMask::MERGE,
            MetricsMask::LATENCY | MetricsMask::MEMORY,
        );
        assert_eq!(c[0].mask(), MetricsMask::LATENCY | MetricsMask::MEMORY);
    }
}
