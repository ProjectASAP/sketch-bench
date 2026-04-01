use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const CSV_HEADER: &str = "implementation,language,seed,rows,cols,total_items,distinct_items,avg_relative_error,max_relative_error,mean_absolute_error";
pub const KEY_ERROR_CSV_HEADER: &str =
    "implementation,language,rows,cols,key,true_count,median_estimate,median_relative_error";
pub const KEY_SEED_ERROR_CSV_HEADER: &str =
    "implementation,language,seed,rows,cols,key,true_count,estimate,relative_error";

#[derive(Clone, Debug)]
pub struct AccuracyRow {
    pub implementation: &'static str,
    pub language: &'static str,
    pub seed: u64,
    pub rows: usize,
    pub cols: usize,
    pub total_items: usize,
    pub distinct_items: usize,
    pub avg_relative_error: f64,
    pub max_relative_error: f64,
    pub mean_absolute_error: f64,
}

impl AccuracyRow {
    pub fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.12},{:.12},{:.12}",
            self.implementation,
            self.language,
            self.seed,
            self.rows,
            self.cols,
            self.total_items,
            self.distinct_items,
            self.avg_relative_error,
            self.max_relative_error,
            self.mean_absolute_error
        )
    }
}

#[derive(Clone, Debug)]
pub struct KeyMedianErrorRow {
    pub implementation: &'static str,
    pub language: &'static str,
    pub rows: usize,
    pub cols: usize,
    pub key: i64,
    pub true_count: u64,
    pub median_estimate: u64,
    pub median_relative_error: f64,
}

impl KeyMedianErrorRow {
    pub fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.12}",
            self.implementation,
            self.language,
            self.rows,
            self.cols,
            self.key,
            self.true_count,
            self.median_estimate,
            self.median_relative_error
        )
    }
}

#[derive(Clone, Debug)]
pub struct KeySeedErrorRow {
    pub implementation: &'static str,
    pub language: &'static str,
    pub seed: u64,
    pub rows: usize,
    pub cols: usize,
    pub key: i64,
    pub true_count: u64,
    pub estimate: u64,
    pub relative_error: f64,
}

impl KeySeedErrorRow {
    pub fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{:.12}",
            self.implementation,
            self.language,
            self.seed,
            self.rows,
            self.cols,
            self.key,
            self.true_count,
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

pub struct KeyErrorCsvWriter {
    writer: BufWriter<std::fs::File>,
}

impl KeyErrorCsvWriter {
    pub fn create(path: &Path, append: bool) -> std::io::Result<Self> {
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
            writeln!(writer, "{KEY_ERROR_CSV_HEADER}")?;
        }
        Ok(Self { writer })
    }

    pub fn write_row(&mut self, row: &KeyMedianErrorRow) -> std::io::Result<()> {
        writeln!(self.writer, "{}", row.to_csv_line())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

pub struct KeySeedErrorCsvWriter {
    writer: BufWriter<std::fs::File>,
}

impl KeySeedErrorCsvWriter {
    pub fn create(path: &Path, append: bool) -> std::io::Result<Self> {
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
            writeln!(writer, "{KEY_SEED_ERROR_CSV_HEADER}")?;
        }
        Ok(Self { writer })
    }

    pub fn write_row(&mut self, row: &KeySeedErrorRow) -> std::io::Result<()> {
        writeln!(self.writer, "{}", row.to_csv_line())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
