//! Replaying a dataset off disk.
//!
//! `aqpbm-datagen` writes no files and reads none, so the file end of the round
//! trip lives in core: [`crate::binfile`] writes `.bin` and its sidecar, and
//! this module reads. Three formats, chosen by extension, all yielding `i64` —
//! `.bin` is the tool's own, `.pcap` and `.csv` are how an external trace gets
//! in.

use std::path::Path;

use aqpbm_datagen::DataGenError;

use super::{DatasetDescription, NumericDataset};
use crate::binfile;

impl NumericDataset<i64> {
    /// Load from a file, format from the extension: `.bin` (and anything else)
    /// is a little-endian `int64` stream; `.pcap` takes each IPv4 source address
    /// as big-endian `u32`; `.csv` parses column 0 below a header row.
    pub fn load(path: &Path) -> Result<Self, DataGenError> {
        let items = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("pcap") => load_pcap(path)?,
            Some("csv") => load_csv(path)?,
            _ => {
                reject_non_i64_bin(path)?;
                load_bin(path)?
            }
        };
        if items.is_empty() {
            return Err(DataGenError::BadParam(format!(
                "file contained zero items: {}",
                path.display()
            )));
        }
        Ok(Self::new(
            items,
            DatasetDescription {
                shape: "file".into(),
                size: 0, // overwritten by `new`
                cardinality: None,
                zipf_s: None,
                source_path: Some(path.display().to_string()),
                seed: None,
                spec: None,
            },
        ))
    }
}

/// Reject a `.bin` whose sidecar declares an unreadable dtype. The stream is
/// header-less, so an `f64` file is byte-indistinguishable from `i64` — `0.093`
/// reads back as `4591388162153532928`. No sidecar means "assume i64".
fn reject_non_i64_bin(path: &Path) -> Result<(), DataGenError> {
    let Ok(Some(meta)) = binfile::read_meta(path) else {
        return Ok(());
    };
    if meta.dtype != "i64" {
        return Err(DataGenError::BadParam(format!(
            "{}: sidecar declares dtype {}, but the benchmark only consumes i64. \
             Re-generate with `--dtype i64`; reading it as i64 would silently \
             reinterpret the raw bytes and produce meaningless keys.",
            path.display(),
            meta.dtype,
        )));
    }
    Ok(())
}

fn load_bin(path: &Path) -> Result<Vec<i64>, DataGenError> {
    let bytes = std::fs::read(path).map_err(DataGenError::Io)?;
    if bytes.len() % 8 != 0 {
        return Err(DataGenError::BadParam(format!(
            "{}: size {} not a multiple of 8",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn load_csv(path: &Path) -> Result<Vec<i64>, DataGenError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(DataGenError::Io)?;
    let mut items = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(DataGenError::Io)?;
        if idx == 0 {
            // Header row.
            continue;
        }
        let field = line.split(',').next().unwrap_or("").trim();
        if field.is_empty() {
            continue;
        }
        let v: i64 = field.parse().map_err(|e| {
            DataGenError::BadParam(format!(
                "{}: bad i64 on line {}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        items.push(v);
    }
    Ok(items)
}

fn load_pcap(path: &Path) -> Result<Vec<i64>, DataGenError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(DataGenError::Io)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(DataGenError::Io)?;
    if buf.len() < 24 {
        return Err(DataGenError::BadParam(format!(
            "{}: pcap smaller than header",
            path.display()
        )));
    }
    let big_endian = match &buf[..4] {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => false,
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => true,
        _ => {
            return Err(DataGenError::BadParam(format!(
                "{}: unsupported pcap magic",
                path.display()
            )))
        }
    };
    let read_u32 = |b: &[u8]| {
        let a = [b[0], b[1], b[2], b[3]];
        if big_endian {
            u32::from_be_bytes(a)
        } else {
            u32::from_le_bytes(a)
        }
    };
    let linktype = read_u32(&buf[20..24]);
    let mut off = 24usize;
    let mut items = Vec::new();
    while off + 16 <= buf.len() {
        let incl_len = read_u32(&buf[off + 8..off + 12]) as usize;
        off += 16;
        if off + incl_len > buf.len() {
            return Err(DataGenError::BadParam(format!(
                "{}: truncated pcap record",
                path.display()
            )));
        }
        let pkt = &buf[off..off + incl_len];
        off += incl_len;
        if let Some(src) = extract_ipv4_src(pkt, linktype) {
            items.push(i64::from(src));
        }
    }
    Ok(items)
}

fn extract_ipv4_src(packet: &[u8], linktype: u32) -> Option<u32> {
    let ip = match linktype {
        1 => {
            // Ethernet: 14-byte header, require ethertype = 0x0800.
            if packet.len() < 34 || packet[12] != 0x08 || packet[13] != 0x00 {
                return None;
            }
            &packet[14..]
        }
        101 => packet,
        _ => return None,
    };
    if ip.len() < 20 || (ip[0] >> 4) != 4 {
        return None;
    }
    Some(u32::from_be_bytes([ip[12], ip[13], ip[14], ip[15]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Dataset, I64Dataset};
    use crate::test_support::{build, zipf_column};
    use aqpbm_datagen::TableDescription;

    #[test]
    fn file_bin_roundtrip() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("sketchlib_bin_roundtrip.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        for v in [1i64, -2, 3, 4] {
            f.write_all(&v.to_le_bytes()).unwrap();
        }
        drop(f);
        let w = I64Dataset::load(&path).unwrap();
        assert_eq!(w.items(), &[1, -2, 3, 4]);
        assert_eq!(w.description().shape, "file");
        assert_eq!(w.description().size, 4);
        std::fs::remove_file(&path).ok();
    }

    /// Write a `.bin` + sidecar at `data_type` and try to load it as i64.
    fn load_generated(data_type: &str, tag: &str) -> Result<I64Dataset, DataGenError> {
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = TableDescription::single("key", zipf_column(64, 1.0, 1, data_type), 32);
        let column = spec.generate().unwrap().into_column(0).unwrap();
        binfile::write_bin(&path, &column).unwrap();
        binfile::write_meta(&path, &binfile::BinMeta::new(&spec, &column)).unwrap();
        let out = I64Dataset::load(&path);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(binfile::sidecar_path(&path)).ok();
        out
    }

    #[test]
    fn bin_with_i64_sidecar_loads() {
        let w = load_generated("i64", "i64").expect("i64 must load");
        assert_eq!(w.items().len(), 32);
    }

    #[test]
    fn bin_with_non_i64_sidecar_is_rejected() {
        // An f64/u64 stream is byte-indistinguishable from i64, so
        // loading it would silently produce garbage keys rather than
        // fail. The sidecar is the only thing that can catch it.
        for tag in ["f64", "u64"] {
            let err = load_generated(tag, tag)
                .expect_err("a non-i64 stream must be rejected, not silently misread");
            let msg = err.to_string();
            assert!(
                msg.contains(tag),
                "error should name the offending item type: {msg}"
            );
        }
    }

    #[test]
    fn bin_without_sidecar_is_assumed_i64() {
        // Legacy `input/benchmark_data_*.bin` files have no sidecar and
        // must keep loading unchanged.
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_no_sidecar.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        for v in [7i64, 8, 9] {
            f.write_all(&v.to_le_bytes()).unwrap();
        }
        drop(f);
        let w = I64Dataset::load(&path).expect("no sidecar => assume i64");
        assert_eq!(w.items(), &[7, 8, 9]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unreadable_sidecar_does_not_break_a_loadable_bin() {
        // A foreign `.meta.json`, or one from a future schema, must not
        // fail a file that loaded fine before the guard existed.
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_bad_sidecar.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&5i64.to_le_bytes()).unwrap();
        drop(f);
        std::fs::write(binfile::sidecar_path(&path), "{\"not\":\"ours\"}").unwrap();
        let w = I64Dataset::load(&path).expect("unparseable sidecar => fall back to i64");
        assert_eq!(w.items(), &[5]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(binfile::sidecar_path(&path)).ok();
    }

    /// `dataset generate` (to a file) and `sketchbench --spec` (in memory) must
    /// be the same dataset, or a run cannot be reproduced from its own file.
    #[test]
    fn the_file_and_the_memory_path_agree() {
        let spec = TableDescription::single("key", zipf_column(500, 1.3, 7, "i64"), 3000);
        let path = std::env::temp_dir().join("sketchlib_file_agreement.bin");

        let column = spec.generate().unwrap().into_column(0).unwrap();
        binfile::write_bin(&path, &column).unwrap();

        let from_file = I64Dataset::load(&path).unwrap();
        let from_memory: I64Dataset = build(&spec).unwrap();
        assert_eq!(from_file.items(), from_memory.items());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_csv_skips_header_and_empty() {
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_csv_roundtrip.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "key,value").unwrap();
        writeln!(f, "10,first").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "-5,second").unwrap();
        drop(f);
        let w = I64Dataset::load(&path).unwrap();
        assert_eq!(w.items(), &[10, -5]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_pcap_rejects_empty_or_bad_magic() {
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_pcap_bad.pcap");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0u8; 32]).unwrap();
        drop(f);
        let err = I64Dataset::load(&path).unwrap_err();
        assert!(err.to_string().contains("pcap magic"));
        std::fs::remove_file(&path).ok();
    }
}
