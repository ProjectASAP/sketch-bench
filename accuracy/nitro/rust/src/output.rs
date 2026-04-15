use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const CSV_HEADER: &str = "implementation,language,trial,rows,cols,rate,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error";

#[derive(Clone, Debug)]
pub struct AccuracyRow {
    pub implementation: &'static str,
    pub language: &'static str,
    pub trial: u64,
    pub rows: usize,
    pub cols: usize,
    pub rate: f64,
    pub total_items: usize,
    pub distinct_items: usize,
    pub avg_relative_error: f64,
    pub max_relative_error: f64,
    pub mean_absolute_error: f64,
}

impl AccuracyRow {
    pub fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{:.6},{},{},{:.12},{:.12},{:.12}",
            self.implementation,
            self.language,
            self.trial,
            self.rows,
            self.cols,
            self.rate,
            self.total_items,
            self.distinct_items,
            self.avg_relative_error,
            self.max_relative_error,
            self.mean_absolute_error
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
