use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use sketch_bench::baselines::ExactQuantile;

/// DDSketch and KLL both answer quantile queries, so this harness
/// shares the same ground-truth algorithm with the KLL harness —
/// `sketch_bench::baselines::ExactQuantile`.
#[derive(Debug)]
pub struct BaselineData {
    pub values: Vec<i64>,
    exact: ExactQuantile,
}

impl BaselineData {
    pub fn total_items(&self) -> usize {
        self.values.len()
    }

    pub fn ground_truth_quantile(&self, p: usize) -> f64 {
        self.exact.ground_truth_quantile(p)
    }
}

pub fn load_baseline(path: &Path) -> Result<BaselineData, Box<dyn Error>> {
    let values = load_i64_stream(path)?;
    let exact = ExactQuantile::ingest_all(&values);
    Ok(BaselineData { values, exact })
}

fn load_i64_stream(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len() as usize;
    if file_size == 0 {
        return Err(format!("dataset is empty: {}", path.display()).into());
    }
    if file_size % std::mem::size_of::<i64>() != 0 {
        return Err(format!(
            "dataset size is not divisible by 8 bytes: {} ({} bytes)",
            path.display(),
            file_size
        )
        .into());
    }

    let mut buffer = vec![0u8; file_size];
    file.read_exact(&mut buffer)?;

    let mut values = Vec::with_capacity(file_size / 8);
    for chunk in buffer.chunks_exact(8) {
        values.push(i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]));
    }

    if values.is_empty() {
        return Err(format!("dataset contains zero values: {}", path.display()).into());
    }

    Ok(values)
}
