//! Discard-everything exporter. Useful as a placeholder in
//! tests or when the `sample_every_n` dial is set to zero.

use sketch_core::report::Record;

use super::Exporter;

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopExporter;

impl Exporter for NoopExporter {
    #[inline(always)]
    fn export(&self, _record: &Record) {}
}
