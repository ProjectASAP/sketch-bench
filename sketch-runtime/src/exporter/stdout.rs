//! JSONL → stdout exporter. One line per record.

use std::io::{self, Write};
use std::sync::Mutex;

use aqpbm_core::report::Record;

use super::Exporter;

pub struct StdoutExporter {
    /// Locks the write so overlapping exports from different
    /// threads don't interleave JSONL lines.
    lock: Mutex<()>,
}

impl StdoutExporter {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }
}

impl Default for StdoutExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Exporter for StdoutExporter {
    fn export(&self, record: &Record) {
        let line = record.to_jsonl();
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        // Best-effort — a failed write must not kill the host.
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }
}
