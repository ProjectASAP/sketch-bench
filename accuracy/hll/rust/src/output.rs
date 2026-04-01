use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const CSV_HEADER: &str =
    "implementation,language,seed,lg_k,registers,total_items,true_distinct,estimate,relative_error";

#[derive(Clone, Debug)]
pub struct AccuracyRow {
    pub implementation: &'static str,
    pub language: &'static str,
    pub seed: u64,
    pub lg_k: u8,
    pub registers: usize,
    pub total_items: usize,
    pub true_distinct: usize,
    pub estimate: f64,
    pub relative_error: f64,
}

impl AccuracyRow {
    pub fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.12},{:.12}",
            self.implementation,
            self.language,
            self.seed,
            self.lg_k,
            self.registers,
            self.total_items,
            self.true_distinct,
            self.estimate,
            self.relative_error
        )
    }
}

pub fn write_csv(path: &Path, rows: &[AccuracyRow], append: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let write_header = !append || !path.exists() || path.metadata()?.len() == 0;
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)?;
    let mut writer = BufWriter::new(file);

    if write_header {
        writeln!(writer, "{CSV_HEADER}")?;
    }
    for row in rows {
        writeln!(writer, "{}", row.to_csv_line())?;
    }
    writer.flush()
}
