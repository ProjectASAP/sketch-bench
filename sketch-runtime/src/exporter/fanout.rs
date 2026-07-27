//! `FanOutExporter` — route every record to two `Exporter`s
//! simultaneously. The canonical paper setup: one branch pushes
//! to the controller for real-time decisions (`PushExporter`);
//! the other writes the same records to a local file
//! (`FileExporter`) for offline paper-artifact reproducibility.
//!
//! Composition rather than a variadic list: two-wide covers the
//! paper story, and nesting (`FanOutExporter<A, FanOutExporter<B, C>>`)
//! is a pass-through one-liner if more branches are ever needed.

use aqpbm_core::report::Record;

use super::Exporter;

/// Send each record to both `a` and `b`. Exporters run
/// serially on the caller's thread; `PushExporter` is
/// non-blocking (fire-and-forget via bounded channel), so the
/// only real work done on this path is `FileExporter`'s
/// `write + flush`. A slow branch cannot block the other —
/// each `Exporter::export` is independent.
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
        use aqpbm_core::workload::WorkloadDesc;
        Record::new(
            "hll",
            "oxide",
            WorkloadDesc {
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
