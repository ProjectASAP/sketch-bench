//! Synthetic workload generators + adapters for file-backed
//! test data.
//!
//! See `docs/DESIGN.md` §4.3.

use rand::SeedableRng;
use rand_distr::{Distribution, Uniform, Zipf};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SketchCoreError;

/// Human-friendly description of a workload — serialised into
/// every report so a JSONL record can be re-run without
/// external metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadDesc {
    pub shape: String, // "uniform" | "zipf" | "file"
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zipf_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// The abstract contract for a workload a `BenchRunner` can
/// consume. An implementation produces an ordered `Vec<Item>`
/// plus an optional query stream.
pub trait Workload {
    type Item: Clone;
    fn desc(&self) -> WorkloadDesc;
    fn items(&self) -> &[Self::Item];
}

// ---------- i64 workloads ----------

/// Uniform `i64` workload in `[0, cardinality)`.
#[derive(Debug, Clone)]
pub struct UniformI64 {
    items: Vec<i64>,
    cardinality: u64,
    seed: u64,
}

impl UniformI64 {
    pub fn new(size: usize, cardinality: u64, seed: u64) -> Self {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let dist = Uniform::new(0u64, cardinality);
        let items = (0..size).map(|_| dist.sample(&mut rng) as i64).collect();
        Self {
            items,
            cardinality,
            seed,
        }
    }
}

impl Workload for UniformI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "uniform".into(),
            size: self.items.len(),
            cardinality: Some(self.cardinality),
            zipf_s: None,
            source_path: None,
            seed: Some(self.seed),
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

/// Zipfian `i64` workload with `s`-parameter (skew exponent)
/// over keys `[1, cardinality]`.
#[derive(Debug, Clone)]
pub struct ZipfI64 {
    items: Vec<i64>,
    cardinality: u64,
    s: f64,
    seed: u64,
}

impl ZipfI64 {
    pub fn new(size: usize, cardinality: u64, s: f64, seed: u64) -> Result<Self, SketchCoreError> {
        let dist = Zipf::new(cardinality, s)
            .map_err(|e| SketchCoreError::BadParam(format!("zipf: {e}")))?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let items = (0..size).map(|_| dist.sample(&mut rng) as i64).collect();
        Ok(Self {
            items,
            cardinality,
            s,
            seed,
        })
    }
}

impl Workload for ZipfI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "zipf".into(),
            size: self.items.len(),
            cardinality: Some(self.cardinality),
            zipf_s: Some(self.s),
            source_path: None,
            seed: Some(self.seed),
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

/// File-backed `i64` workload. Dispatches on extension:
///
/// * `.bin` (or anything else) — little-endian `int64` stream,
///   matching the pre-existing `input/benchmark_data_*.bin`
///   layout.
/// * `.pcap` — libpcap capture. For each IPv4 packet the source
///   address is read as a big-endian `u32` and sign-extended into
///   an `i64`. Non-IPv4 packets are skipped. Used by the
///   frequency-family accuracy harness against network traces.
/// * `.csv` — CSV with a header row; the first column on every
///   subsequent row is parsed as `i64`. Empty lines skipped.
#[derive(Debug, Clone)]
pub struct FileI64 {
    items: Vec<i64>,
    source_path: String,
}

impl FileI64 {
    /// Auto-detect format from the file extension and load.
    pub fn load(path: &Path) -> Result<Self, SketchCoreError> {
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
            return Err(SketchCoreError::BadParam(format!(
                "file contained zero items: {}",
                path.display()
            )));
        }
        Ok(Self {
            items,
            source_path: path.display().to_string(),
        })
    }
}

/// Reject a `.bin` whose sidecar declares a dtype this loader cannot
/// read.
///
/// The `.bin` stream is header-less, so it cannot describe itself: a
/// `u64`/`f64` file is byte-indistinguishable from an `i64` one and
/// [`load_bin`] would happily reinterpret every 8-byte word as an
/// `i64`. For `f64` that is catastrophic — the IEEE-754 bit pattern of
/// `0.093` reads back as `4591388162153532928` — and the run would
/// still emit a well-formed, plausible-looking report. A benchmark
/// number that is silently wrong is worse than no number at all, so
/// this fails loudly instead.
///
/// Files with no sidecar (every legacy `input/benchmark_data_*.bin`)
/// are accepted unchanged: absence of provenance means "assume i64",
/// which is the historical contract.
fn reject_non_i64_bin(path: &Path) -> Result<(), SketchCoreError> {
    let Some(meta) = crate::datagen::io::read_meta(path)? else {
        return Ok(());
    };
    if meta.dtype != crate::datagen::DType::I64 {
        return Err(SketchCoreError::BadParam(format!(
            "{}: sidecar declares dtype {}, but the benchmark only consumes i64. \
             Re-generate with `--dtype i64`; reading it as i64 would silently \
             reinterpret the raw bytes and produce meaningless keys.",
            path.display(),
            meta.dtype.as_str(),
        )));
    }
    Ok(())
}

fn load_bin(path: &Path) -> Result<Vec<i64>, SketchCoreError> {
    let bytes = std::fs::read(path).map_err(SketchCoreError::Io)?;
    if bytes.len() % 8 != 0 {
        return Err(SketchCoreError::BadParam(format!(
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

fn load_csv(path: &Path) -> Result<Vec<i64>, SketchCoreError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(SketchCoreError::Io)?;
    let mut items = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(SketchCoreError::Io)?;
        if idx == 0 {
            // Header row.
            continue;
        }
        let field = line.split(',').next().unwrap_or("").trim();
        if field.is_empty() {
            continue;
        }
        let v: i64 = field.parse().map_err(|e| {
            SketchCoreError::BadParam(format!(
                "{}: bad i64 on line {}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        items.push(v);
    }
    Ok(items)
}

fn load_pcap(path: &Path) -> Result<Vec<i64>, SketchCoreError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(SketchCoreError::Io)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(SketchCoreError::Io)?;
    if buf.len() < 24 {
        return Err(SketchCoreError::BadParam(format!(
            "{}: pcap smaller than header",
            path.display()
        )));
    }
    let big_endian = match &buf[..4] {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => false,
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => true,
        _ => {
            return Err(SketchCoreError::BadParam(format!(
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
            return Err(SketchCoreError::BadParam(format!(
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

impl Workload for FileI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "file".into(),
            size: self.items.len(),
            cardinality: None,
            zipf_s: None,
            source_path: Some(self.source_path.clone()),
            seed: None,
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

// ---------- derived workloads for string / bytes impls ----------

/// Derive a `String` workload from any `i64` workload by
/// decimal-formatting each item. Lets the existing
/// `Elastic`/`UnivMon` string sketches reuse the same
/// distributions without forking the generators.
#[derive(Debug, Clone)]
pub struct StringFromI64<W: Workload<Item = i64>> {
    items: Vec<String>,
    inner_desc: WorkloadDesc,
    _marker: std::marker::PhantomData<W>,
}

impl<W: Workload<Item = i64>> StringFromI64<W> {
    pub fn new(inner: &W) -> Self {
        let items = inner.items().iter().map(|v| v.to_string()).collect();
        Self {
            items,
            inner_desc: inner.desc(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<W: Workload<Item = i64>> Workload for StringFromI64<W> {
    type Item = String;
    fn desc(&self) -> WorkloadDesc {
        self.inner_desc.clone()
    }
    fn items(&self) -> &[String] {
        &self.items
    }
}

/// Same, but `Vec<u8>` for impls that want `&[u8]`.
#[derive(Debug, Clone)]
pub struct BytesFromI64<W: Workload<Item = i64>> {
    items: Vec<Vec<u8>>,
    inner_desc: WorkloadDesc,
    _marker: std::marker::PhantomData<W>,
}

impl<W: Workload<Item = i64>> BytesFromI64<W> {
    pub fn new(inner: &W) -> Self {
        let items = inner
            .items()
            .iter()
            .map(|v| v.to_string().into_bytes())
            .collect();
        Self {
            items,
            inner_desc: inner.desc(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<W: Workload<Item = i64>> Workload for BytesFromI64<W> {
    type Item = Vec<u8>;
    fn desc(&self) -> WorkloadDesc {
        self.inner_desc.clone()
    }
    fn items(&self) -> &[Vec<u8>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_is_reproducible_from_seed() {
        let a = UniformI64::new(100, 1000, 42);
        let b = UniformI64::new(100, 1000, 42);
        assert_eq!(a.items(), b.items());
    }

    #[test]
    fn zipf_items_in_expected_range() {
        let w = ZipfI64::new(1000, 100, 1.1, 7).unwrap();
        for v in w.items() {
            assert!(*v >= 1 && *v <= 100);
        }
    }

    #[test]
    fn string_workload_derived_length() {
        let inner = UniformI64::new(50, 100, 1);
        let s = StringFromI64::new(&inner);
        assert_eq!(s.items().len(), 50);
        assert_eq!(s.desc().shape, "uniform");
    }

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
        let w = FileI64::load(&path).unwrap();
        assert_eq!(w.items(), &[1, -2, 3, 4]);
        assert_eq!(w.desc().shape, "file");
        std::fs::remove_file(&path).ok();
    }

    /// Generate a `.bin` + sidecar of the given dtype and try to load it.
    fn load_generated(dtype: crate::datagen::DType, tag: &str) -> Result<FileI64, SketchCoreError> {
        use crate::datagen::{io, GenMeta, GenSpec, Shape};
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = GenSpec {
            shape: Shape::Uniform {
                cardinality: 64,
                dtype,
            },
            size: 32,
            seed: 1,
        };
        let col = spec.generate().unwrap();
        io::write_bin(&path, &col).unwrap();
        io::write_meta(&path, &GenMeta::new(&spec, &col)).unwrap();
        let out = FileI64::load(&path);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(io::sidecar_path(&path)).ok();
        out
    }

    #[test]
    fn bin_with_i64_sidecar_loads() {
        let w = load_generated(crate::datagen::DType::I64, "i64").expect("i64 must load");
        assert_eq!(w.items().len(), 32);
    }

    #[test]
    fn bin_with_non_i64_sidecar_is_rejected() {
        // An f64/u64 stream is byte-indistinguishable from i64, so
        // loading it would silently produce garbage keys rather than
        // fail. The sidecar is the only thing that can catch it.
        for (dtype, tag) in [
            (crate::datagen::DType::F64, "f64"),
            (crate::datagen::DType::U64, "u64"),
        ] {
            let err = load_generated(dtype, tag)
                .expect_err("non-i64 dtype must be rejected, not silently misread");
            let msg = err.to_string();
            assert!(msg.contains(tag), "error should name the offending dtype: {msg}");
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
        let w = FileI64::load(&path).expect("no sidecar => assume i64");
        assert_eq!(w.items(), &[7, 8, 9]);
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
        let w = FileI64::load(&path).unwrap();
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
        let err = FileI64::load(&path).unwrap_err();
        assert!(err.to_string().contains("pcap magic"));
        std::fs::remove_file(&path).ok();
    }
}
