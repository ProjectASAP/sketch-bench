use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct BaselineData {
    pub values: Vec<i64>,
    pub frequencies: HashMap<i64, u64>,
}

impl BaselineData {
    pub fn total_items(&self) -> usize {
        self.values.len()
    }

    pub fn distinct_items(&self) -> usize {
        self.frequencies.len()
    }
}

pub fn load_baseline(path: &Path) -> Result<BaselineData, Box<dyn Error>> {
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
    let mut frequencies = HashMap::new();
    for chunk in buffer.chunks_exact(8) {
        let value = i64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        values.push(value);
        *frequencies.entry(value).or_insert(0) += 1;
    }

    if values.is_empty() {
        return Err(format!("dataset contains zero values: {}", path.display()).into());
    }
    if frequencies.is_empty() {
        return Err(format!("baseline contains zero distinct keys: {}", path.display()).into());
    }

    Ok(BaselineData {
        values,
        frequencies,
    })
}
