use std::fs::OpenOptions;
use std::io::Write;

use anyhow::Result;

use crate::repeat;

/// Open the `--report` destination. `None` or `"-"` → stdout.
pub enum ReportSink {
    Stdout,
    File(std::fs::File),
}

impl ReportSink {
    pub fn open(spec: Option<&str>) -> Result<Self> {
        if repeat::is_child() {
            return Ok(ReportSink::Stdout);
        }
        match spec {
            None | Some("-") => Ok(ReportSink::Stdout),
            Some(path) => Ok(ReportSink::File(
                OpenOptions::new().create(true).append(true).open(path)?,
            )),
        }
    }
    pub fn write_line(&mut self, line: &str) -> Result<()> {
        match self {
            ReportSink::Stdout => {
                println!("{line}");
                Ok(())
            }
            ReportSink::File(f) => {
                f.write_all(line.as_bytes())?;
                f.write_all(b"\n")?;
                Ok(())
            }
        }
    }
}
