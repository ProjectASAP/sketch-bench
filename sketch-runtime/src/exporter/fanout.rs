//! `FanOutExporter` — route every record to two `Exporter`s at once: one branch
//! pushes to the controller for real-time decisions, the other writes the same
//! records to a local file for offline reproducibility. Composition rather than
//! a variadic list — nesting covers more branches in a one-liner.

use aqpbm_core::report::Record;

use super::Exporter;

/// Send each record to both `a` and `b`, serially on the caller's thread. The
/// push branch is fire-and-forget via a bounded channel, so the only real work
/// here is the file branch's `write + flush`.
pub struct FanOutExporter<A: Exporter, B: Exporter> {
    pub a: A,
    pub b: B,
}

impl<A: Exporter, B: Exporter> FanOutExporter<A, B> {
    pub fn new(a: A, b: B) -> Self {
        Self { a, b }
    }
}

impl<A: Exporter, B: Exporter> Exporter for FanOutExporter<A, B> {
    fn export(&self, record: &Record) {
        self.a.export(record);
        self.b.export(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CapturingExporter {
        records: Arc<Mutex<Vec<Record>>>,
    }
    impl CapturingExporter {
        fn count(&self) -> usize {
            self.records.lock().unwrap().len()
        }
        fn shared(&self) -> Self {
            Self {
                records: self.records.clone(),
            }
        }
    }
    impl Exporter for CapturingExporter {
        fn export(&self, record: &Record) {
            self.records.lock().unwrap().push(record.clone());
        }
    }

    fn sample_record() -> Record {
        use aqpbm_core::report::Mode;
        use aqpbm_core::workload::WorkloadDescription;
        Record::new(
            "hll",
            "oxide",
            WorkloadDescription {
                shape: "test".into(),
                size: 1,
                cardinality: None,
                zipf_s: None,
                source_path: None,
                seed: None,
                spec: None,
            },
            Mode::Runtime,
            1,
        )
    }

    #[test]
    fn fans_out_one_record_to_two_exporters() {
        let a = CapturingExporter::default();
        let b = CapturingExporter::default();
        let (a_shared, b_shared) = (a.shared(), b.shared());
        let fan = FanOutExporter::new(a, b);
        fan.export(&sample_record());
        assert_eq!(a_shared.count(), 1);
        assert_eq!(b_shared.count(), 1);
    }
}
