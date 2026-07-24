//! Writes the legacy CSV files for `sketchlib bench --raw-csv DIR`.
//!
//! The *content* — headers, family-specific columns, filenames — is rendered
//! by [`sketch_bench::legacy_csv`]. This is only the sink: create the
//! directory and append each rendered file, mirroring how `ReportSink` writes
//! the JSONL that the `Record` schema produces.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use aqpbm_datagen::DType;
use sketch_bench::dispatch::ImplEntry;
use sketch_bench::legacy_csv;
use sketch_bench::params::ParamSet;
use sketch_bench::runner::BenchReport;

pub fn write_runs(
    dir: &Path,
    entry: &ImplEntry,
    params: Option<&ParamSet>,
    seed: u64,
    workers: usize,
    dtype: DType,
    report: &BenchReport,
) -> Result<()> {
    let files = legacy_csv::render(entry, params, seed, workers, dtype, report);
    if files.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for file in &files {
        append_csv(&dir.join(&file.name), &file.header, &file.rows)?;
    }
    Ok(())
}

/// Append `rows` to `path`, writing `header` first only when the file is new.
/// `--raw-csv` appends, so a file that already exists keeps accumulating and a
/// run lands in whatever is there.
fn append_csv(path: &Path, header: &str, rows: &[String]) -> Result<()> {
    let new_file = !path.exists();
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    if new_file {
        writeln!(f, "{header}")?;
    }
    for row in rows {
        writeln!(f, "{row}")?;
    }
    Ok(())
}
