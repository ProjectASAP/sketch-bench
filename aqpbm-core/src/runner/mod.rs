//! Shaping one measurement into a record: [`BenchReport`], and the fold from a
//! measurement's per-run metrics into a [`BenchSection`]. The running itself is
//! [`measure()`](fn@crate::measure), which times a closure and knows nothing else.

use crate::dataset::DatasetDescription;
use crate::metrics::{Metric, Operation, RunMetrics};
use crate::report::{BenchSection, Mode, Record, Source};
use crate::run_stats;

/// One measurement, ready to become a record.
pub struct BenchReport {
    pub sketch: String,
    pub impl_name: String,
    pub dataset: DatasetDescription,
    pub bench: BenchSection,
    /// Measured iterations, for the record's `runs` field.
    pub runs: usize,
}

impl BenchReport {
    /// Fold a measurement's runs into a report, labelled by the square it was.
    ///
    /// The caller says which operation and metric this was; core does not
    /// decide, it records. Which `BenchSection` slot the rate lands in follows
    /// from the operation, because that is what the field names mean.
    pub fn fold(
        sketch: impl Into<String>,
        impl_name: impl Into<String>,
        dataset: DatasetDescription,
        operation: Operation,
        metric: Metric,
        runs: Vec<RunMetrics>,
    ) -> Self {
        let mut bench = BenchSection {
            operation: Some(operation.name().to_string()),
            metric: Some(metric.name().to_string()),
            ..Default::default()
        };

        // The rate this measurement produced, under the name that operation's
        // rate carries in the schema.
        match operation {
            Operation::Insert => {
                bench.throughput_items_per_sec = run_stats::rate(&runs);
                bench.throughput_samples = run_stats::rate_samples(&runs);
            }
            Operation::Query => bench.query_throughput_items_per_sec = run_stats::rate(&runs),
            Operation::Merge => {
                bench.merge_folds_per_sec = run_stats::rate(&runs);
                bench.merge_time_ms = run_stats::elapsed_ms(&runs);
                bench.merge_supported = Some(true);
            }
            Operation::Prepare => bench.finalize_time_ms = run_stats::elapsed_ms(&runs),
        }
        if metric == Metric::Latency {
            bench.latency_ns = run_stats::latency(&runs);
        }
        if metric == Metric::Accuracy {
            bench.accuracy = run_stats::scores(&runs);
        }

        bench.wall_time_ms = run_stats::elapsed_ms(&runs);
        bench.cpu_time_ms = run_stats::cpu_time_ms(&runs);
        let mem = run_stats::memory_maxima(&runs);
        bench.rss_peak_kb = mem.rss_peak_kb;
        bench.heap_allocated_kb = mem.heap_allocated_kb;
        bench.heap_bytes_net = mem.heap_bytes_net;
        bench.heap_bytes_peak = mem.heap_bytes_peak;
        bench.memory_bytes = run_stats::memory_bytes(&runs);

        Self {
            sketch: sketch.into(),
            impl_name: impl_name.into(),
            dataset,
            runs: runs.len(),
            bench,
        }
    }

    /// Build a v1 JSONL record from this report.
    pub fn to_record(&self) -> Record {
        let mut rec = Record::new(
            self.sketch.clone(),
            self.impl_name.clone(),
            self.dataset.clone(),
            Mode::Bench,
            self.runs,
        );
        rec.bench = Some(self.bench.clone());
        rec.source = Source::Cli;
        rec
    }

    pub fn to_jsonl(&self) -> String {
        self.to_record().to_jsonl()
    }
}
