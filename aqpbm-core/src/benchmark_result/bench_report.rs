//! Shaping one measurement into a record: [`BenchReport`], and the fold from a
//! measurement's per-run metrics into a [`BenchSection`]. The running itself is
//! [`measure()`](fn@crate::measure), which times a closure and knows nothing else.

use crate::benchmark_result::fold;
use crate::benchmark_result::schema::{BenchSection, Mode, Record, Source};
use crate::metrics::{Metric, Operation, RunMetrics};
use aqpbm_datagen::TableDescription;

/// One measurement, ready to become a record.
pub struct BenchReport {
    pub sketch: String,
    pub library: String,
    pub input_dataset: TableDescription,
    pub bench: BenchSection,
    /// Measured iterations, for the record's `runs` field.
    pub runs: usize,
}

impl BenchReport {
    /// Summarise a measurement's runs into a report, labelled by what it measured.
    ///
    /// The caller says which operation and metric this was; core does not
    /// decide, it records. The pair picks the `BenchSection` slot: the operation
    /// names it, and the metric is what entitles it to be written at all.
    pub fn from_runs(
        sketch: impl Into<String>,
        library: impl Into<String>,
        input_dataset: TableDescription,
        operation: Operation,
        metric: Metric,
        runs: Vec<RunMetrics>,
    ) -> Self {
        let mut bench = BenchSection {
            operation: Some(operation.name().to_string()),
            metric: Some(metric.name().to_string()),
            ..Default::default()
        };

        // Only the metric that was asked for writes a number. A rate read off a
        // `time_each` region prices the per-call clock, not the operation.
        match (operation, metric) {
            (Operation::Insert, Metric::Throughput) => {
                bench.throughput_items_per_sec = fold::rate(&runs);
                bench.throughput_samples = fold::rate_samples(&runs);
            }
            (Operation::Query, Metric::Throughput) => {
                bench.query_throughput_items_per_sec = fold::rate(&runs);
            }
            (Operation::Merge, Metric::Throughput) => {
                bench.merge_folds_per_sec = fold::rate(&runs);
            }
            (_, Metric::Latency) => bench.latency_ns = fold::latency(&runs),
            (_, Metric::Accuracy) => bench.accuracy = fold::scores(&runs),
            _ => {}
        }
        if operation == Operation::Merge {
            bench.merge_time_ms = fold::elapsed_ms(&runs);
            bench.merge_supported = Some(true);
        }
        if operation == Operation::Prepare {
            bench.finalize_time_ms = fold::elapsed_ms(&runs);
        }

        bench.wall_time_ms = fold::elapsed_ms(&runs);
        bench.cpu_time_ms = fold::cpu_time_ms(&runs);
        let mem = fold::memory_maxima(&runs);
        bench.rss_peak_kb = mem.rss_peak_kb;
        bench.heap_allocated_kb = mem.heap_allocated_kb;
        bench.heap_bytes_net = mem.heap_bytes_net;
        bench.heap_bytes_peak = mem.heap_bytes_peak;
        bench.memory_bytes = fold::memory_bytes(&runs);

        Self {
            sketch: sketch.into(),
            library: library.into(),
            input_dataset,
            runs: runs.len(),
            bench,
        }
    }

    /// Build a v1 JSONL record from this report.
    pub fn to_record(&self) -> Record {
        let mut rec = Record::new(
            self.sketch.clone(),
            self.library.clone(),
            self.input_dataset.clone(),
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
