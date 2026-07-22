//! JSONL → append-only file exporter.
//!
//! Opens a single `File` at construction time; serialises writes
//! behind a `Mutex`. No rotation, no buffering beyond the OS's —
//! each `export` is a write + flush, trading throughput for
//! crash-resilience. Rotation / buffered mode belong in a later
//! iteration when a production use-case demands them.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use aqpbm_core::report::Record;

use super::Exporter;

pub struct FileExporter {
    inner: Mutex<File>,
}

impl FileExporter {
    /// Open (or create + append) a JSONL file at `path`.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())?;
        Ok(Self {
            inner: Mutex::new(file),
        })
    }
}

impl Exporter for FileExporter {
    fn export(&self, record: &Record) {
        let line = record.to_jsonl();
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let _ = guard.write_all(line.as_bytes());
        let _ = guard.write_all(b"\n");
        // Sync-every-write — cheap on local disk, worth it so a
        // host crash doesn't lose the last window's bench record.
        let _ = guard.flush();
    }
}
