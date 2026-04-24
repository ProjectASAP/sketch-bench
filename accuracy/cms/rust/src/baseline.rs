use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use sketch_bench::baselines::{cms::HEAVY_HITTER_MIN_TRUE_COUNT as SHARED_MIN, ExactCms};

/// Re-exported for any caller that still imports the constant by
/// the old name. Canonical definition lives in
/// `sketch_bench::baselines::cms`.
pub const HEAVY_HITTER_MIN_TRUE_COUNT: u64 = SHARED_MIN;

#[derive(Debug)]
pub struct BaselineData {
    pub values: Vec<i64>,
    exact: ExactCms,
}

impl BaselineData {
    pub fn total_items(&self) -> usize {
        self.values.len()
    }

    pub fn distinct_items(&self) -> usize {
        self.exact.distinct_items()
    }

    pub fn heavy_hitters(&self) -> Vec<(i64, u64)> {
        self.exact.heavy_hitters(HEAVY_HITTER_MIN_TRUE_COUNT)
    }
}

pub fn load_baseline(path: &Path) -> Result<BaselineData, Box<dyn Error>> {
    let values = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pcap"))
    {
        load_pcap_stream(path)?
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
    {
        load_csv_stream(path)?
    } else {
        load_i64_stream(path)?
    };

    let exact = ExactCms::ingest_all(&values);
    if exact.distinct_items() == 0 {
        return Err(format!("baseline contains zero distinct keys: {}", path.display()).into());
    }

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

fn load_pcap_stream(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    if buffer.len() < 24 {
        return Err(format!("pcap file too small: {}", path.display()).into());
    }

    let endianness = detect_pcap_endianness(&buffer[..4])
        .ok_or_else(|| format!("unsupported pcap magic in {}", path.display()))?;
    let linktype = read_u32(&buffer[20..24], endianness);

    let mut offset = 24usize;
    let mut values = Vec::new();

    while offset + 16 <= buffer.len() {
        let incl_len = read_u32(&buffer[offset + 8..offset + 12], endianness)
            .try_into()
            .unwrap_or(0usize);
        offset += 16;
        if offset + incl_len > buffer.len() {
            return Err(format!("truncated packet data in {}", path.display()).into());
        }

        let packet = &buffer[offset..offset + incl_len];
        offset += incl_len;

        if let Some(key) = extract_ipv4_source(packet, linktype) {
            values.push(i64::from(key));
        }
    }

    if values.is_empty() {
        return Err(format!("pcap contains zero IPv4 packets: {}", path.display()).into());
    }

    Ok(values)
}

fn load_csv_stream(path: &Path) -> Result<Vec<i64>, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut values = Vec::new();

    for (line_index, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if line_index == 0 {
            continue;
        }
        let field = line.split(',').next().unwrap_or("").trim();
        if field.is_empty() {
            continue;
        }
        let value: i64 = field.parse().map_err(|e| {
            format!(
                "failed to parse first field as i64 on line {}: {e}",
                line_index + 1
            )
        })?;
        values.push(value);
    }

    if values.is_empty() {
        return Err(format!("CSV contains no data rows: {}", path.display()).into());
    }

    Ok(values)
}

#[derive(Copy, Clone)]
enum Endianness {
    Little,
    Big,
}

fn detect_pcap_endianness(magic: &[u8]) -> Option<Endianness> {
    match magic {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => Some(Endianness::Little),
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => Some(Endianness::Big),
        _ => None,
    }
}

fn read_u32(bytes: &[u8], endianness: Endianness) -> u32 {
    let array = [bytes[0], bytes[1], bytes[2], bytes[3]];
    match endianness {
        Endianness::Little => u32::from_le_bytes(array),
        Endianness::Big => u32::from_be_bytes(array),
    }
}

fn extract_ipv4_source(packet: &[u8], linktype: u32) -> Option<u32> {
    match linktype {
        1 => extract_ipv4_source_ethernet(packet),
        101 => extract_ipv4_source_raw(packet),
        _ => None,
    }
}

fn extract_ipv4_source_ethernet(packet: &[u8]) -> Option<u32> {
    if packet.len() < 34 {
        return None;
    }
    if packet[12] != 0x08 || packet[13] != 0x00 {
        return None;
    }
    extract_ipv4_source_raw(&packet[14..])
}

fn extract_ipv4_source_raw(packet: &[u8]) -> Option<u32> {
    if packet.len() < 20 {
        return None;
    }
    if (packet[0] >> 4) != 4 {
        return None;
    }
    Some(u32::from_be_bytes([
        packet[12], packet[13], packet[14], packet[15],
    ]))
}
