//! Synthetic workload generators + adapters for file-backed
//! test data.
//!
//! See `docs/DESIGN.md` §4.3.
//!
//! ## One type per item type, not one per source
//!
//! Every `i64` workload — generated or file-backed — is the same
//! thing at runtime: an owned `Vec<i64>` plus the [`WorkloadDesc`]
//! that says where it came from. Modelling each *source* as its own
//! `impl Workload` type forced every generic consumer to fan out over
//! the source set (`sketch-cli`'s dispatch table carried a
//! 3-variant `WorkloadAny` plus two derived 3-variant enums, and every
//! dispatch macro repeated its body once per variant). Provenance is
//! data, not a type parameter, so it lives in the `desc` field and the
//! source only picks a constructor.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::datagen::{DType, Distribution, GenSpec, Shape};
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
    /// Full `datagen` spec, when the flat fields above cannot express
    /// the shape (categorical weights, timestamp gap distributions, …).
    /// Absent for `uniform` / `zipf` / `file`, whose flat fields already
    /// round-trip — so records from those paths are byte-identical to
    /// what shipped before the generator was wired into `bench`.
    /// See `Shape::to_workload_desc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
}

/// The abstract contract for a workload a `BenchRunner` can
/// consume. An implementation produces an ordered `Vec<Item>`
/// plus an optional query stream.
pub trait Workload: Sized {
    type Item: Clone;
    fn desc(&self) -> WorkloadDesc;
    fn items(&self) -> &[Self::Item];

    /// An **independent draw** from the same distribution, for a repetition
    /// that must not reuse the previous one.
    ///
    /// Accuracy is a deterministic function of (data, parameters): re-running
    /// a sketch over the same items with the same seed produces the identical
    /// error, so N repetitions over one fixed workload yield N identical
    /// numbers and any spread reported from them is fabricated. Varying the
    /// *data* is what the literature does (Harmouch: 10 independent datasets
    /// per point; Heule: 5000; Ertl and DataSketches: fresh values per trial).
    ///
    /// `None` means this workload has no distribution to redraw from — a file
    /// on disk is one fixed sample. Callers must then run **one** repetition
    /// and report `n = 1`, not N copies of it.
    fn resample(&self, _seed: u64) -> Option<Self> {
        None
    }

    /// Whether [`Self::resample`] can produce anything, without paying for a
    /// generation to find out. The runner needs this *before* it decides how
    /// many repetitions to run, and probing by calling `resample` would
    /// generate a full workload only to discard it.
    fn can_resample(&self) -> bool {
        false
    }
}

// ---------- i64 workloads ----------

/// An `i64` workload: the materialised item stream plus its
/// provenance. Construct it from a generator (`uniform` / `zipf`)
/// or from a file (`load`); the source shows up in `desc`, not in
/// the type.
#[derive(Debug, Clone)]
pub struct I64Workload {
    items: Vec<i64>,
    desc: WorkloadDesc,
    /// The spec this was generated from, when it was generated. Retained so
    /// [`Workload::resample`] can draw again from the same distribution.
    /// `None` for file-backed workloads: a file is one fixed sample.
    spec: Option<GenSpec>,
}

impl I64Workload {
    /// Wrap an already-materialised item stream with its provenance.
    /// `desc.size` is forced to match `items.len()` — a desc that
    /// disagrees with the data it describes would silently corrupt
    /// every throughput denominator downstream.
    pub fn new(items: Vec<i64>, mut desc: WorkloadDesc) -> Self {
        // `load` cannot know the count until it has read the file, so it
        // passes 0 as a placeholder. Any other value is the caller *asserting*
        // what was produced — a generator returning short would otherwise be
        // relabelled into a smaller workload with no signal at all.
        debug_assert!(
            desc.size == 0 || desc.size == items.len(),
            "workload desc claims {} items but carries {}",
            desc.size,
            items.len(),
        );
        desc.size = items.len();
        Self {
            items,
            desc,
            spec: None,
        }
    }

    /// Generate in-process from a [`GenSpec`] — the one generator in
    /// the tool. `sketchlib workload generate` runs the same spec
    /// through a file sink; this runs it through a memory sink, so a
    /// shape reachable on disk is reachable here by construction.
    ///
    /// Fails if the spec's dtype is not `i64`: the benchmark consumes
    /// `i64` keys, and reinterpreting a `u64`/`f64` stream would yield
    /// meaningless keys behind a well-formed report.
    pub fn generate(spec: &GenSpec) -> Result<Self, SketchCoreError> {
        let desc = spec.shape.to_workload_desc(spec.size, spec.seed);
        let mut wk = Self::new(spec.generate()?.into_i64()?, desc);
        wk.spec = Some(spec.clone());
        Ok(wk)
    }

    /// Uniform in `[0, cardinality)`. Convenience over [`Self::generate`].
    pub fn uniform(size: usize, cardinality: u64, seed: u64) -> Self {
        Self::generate(&GenSpec {
            shape: Shape::Keys {
                cardinality,
                dist: Distribution::Uniform,
                dtype: DType::I64,
            },
            size,
            seed,
        })
        .expect("uniform keys over a non-zero cardinality always generate")
    }

    /// Zipfian with `s`-parameter (skew exponent) over ranks
    /// `[1, cardinality]`. Convenience over [`Self::generate`].
    pub fn zipf(size: usize, cardinality: u64, s: f64, seed: u64) -> Result<Self, SketchCoreError> {
        Self::generate(&GenSpec {
            shape: Shape::Keys {
                cardinality,
                dist: Distribution::Zipf { s },
                dtype: DType::I64,
            },
            size,
            seed,
        })
    }

    /// Load from a file, auto-detecting the format from its
    /// extension:
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
        Ok(Self::new(
            items,
            WorkloadDesc {
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

impl Workload for I64Workload {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        self.desc.clone()
    }
    fn items(&self) -> &[i64] {
        &self.items
    }

    /// Regenerate from the retained spec with `seed` substituted. A spec that
    /// generated once cannot fail on a different seed — every validation in
    /// `Shape::build` is seed-independent — so a failure here would be a bug,
    /// and returning `None` degrades to "cannot vary", which the caller
    /// already handles honestly.
    fn can_resample(&self) -> bool {
        self.spec.is_some()
    }

    fn resample(&self, seed: u64) -> Option<Self> {
        let spec = self.spec.as_ref()?;
        let mut respec = spec.clone();
        respec.seed = seed;
        Self::generate(&respec).ok()
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
/// Absence of usable provenance means "assume i64", the historical
/// contract — so a missing sidecar (every legacy
/// `input/benchmark_data_*.bin`) loads unchanged, and an unreadable one
/// (foreign file, or a future schema this binary predates) degrades to
/// the same path rather than failing a file that used to load. Only a
/// sidecar we can actually parse is allowed to veto. `describe`, where
/// the user asked about the sidecar specifically, keeps the strict
/// [`crate::datagen::io::read_meta`] error.
fn reject_non_i64_bin(path: &Path) -> Result<(), SketchCoreError> {
    let Ok(Some(meta)) = crate::datagen::io::read_meta(path) else {
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

// ---------- derived workloads for string / bytes impls ----------

/// A `String` workload derived from an [`I64Workload`] by
/// decimal-formatting each item. Lets the `Elastic`/`UnivMon` string
/// sketches reuse the same distributions without forking the
/// generators. Carries the source workload's `desc` unchanged — the
/// item encoding is a wrapper concern, not workload provenance.
#[derive(Debug, Clone)]
pub struct StringWorkload {
    items: Vec<String>,
    desc: WorkloadDesc,
}

impl StringWorkload {
    pub fn from_i64(inner: &I64Workload) -> Self {
        Self {
            items: inner.items().iter().map(|v| v.to_string()).collect(),
            desc: inner.desc(),
        }
    }
}

impl Workload for StringWorkload {
    type Item = String;
    fn desc(&self) -> WorkloadDesc {
        self.desc.clone()
    }
    fn items(&self) -> &[String] {
        &self.items
    }
}

/// Same, but `Vec<u8>` for impls that want `&[u8]`.
#[derive(Debug, Clone)]
pub struct BytesWorkload {
    items: Vec<Vec<u8>>,
    desc: WorkloadDesc,
}

impl BytesWorkload {
    pub fn from_i64(inner: &I64Workload) -> Self {
        Self {
            items: inner
                .items()
                .iter()
                .map(|v| v.to_string().into_bytes())
                .collect(),
            desc: inner.desc(),
        }
    }
}

impl Workload for BytesWorkload {
    type Item = Vec<u8>;
    fn desc(&self) -> WorkloadDesc {
        self.desc.clone()
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
        let a = I64Workload::uniform(100, 1000, 42);
        let b = I64Workload::uniform(100, 1000, 42);
        assert_eq!(a.items(), b.items());
    }

    #[test]
    fn zipf_items_in_expected_range() {
        let w = I64Workload::zipf(1000, 100, 1.1, 7).unwrap();
        for v in w.items() {
            assert!(*v >= 1 && *v <= 100);
        }
    }

    #[test]
    fn string_workload_derived_length() {
        let inner = I64Workload::uniform(50, 100, 1);
        let s = StringWorkload::from_i64(&inner);
        assert_eq!(s.items().len(), 50);
        assert_eq!(s.desc().shape, "uniform");
    }

    #[test]
    fn placeholder_desc_size_is_filled_in() {
        // A desc that disagrees with the data would silently skew every
        // throughput denominator; `new` is the one place that can catch
        // it, so it always wins over the caller's claim.
        let w = I64Workload::new(
            vec![1, 2, 3],
            WorkloadDesc {
                shape: "custom".into(),
                size: 0, // placeholder, as `load` passes
                cardinality: None,
                zipf_s: None,
                source_path: None,
                seed: None,
                spec: None,
            },
        );
        assert_eq!(w.desc().size, 3);
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
        let w = I64Workload::load(&path).unwrap();
        assert_eq!(w.items(), &[1, -2, 3, 4]);
        assert_eq!(w.desc().shape, "file");
        assert_eq!(w.desc().size, 4);
        std::fs::remove_file(&path).ok();
    }

    /// Generate a `.bin` + sidecar of the given dtype and try to load it.
    fn load_generated(
        dtype: crate::datagen::DType,
        tag: &str,
    ) -> Result<I64Workload, SketchCoreError> {
        use crate::datagen::{io, Distribution, GenMeta, GenSpec, Shape};
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = GenSpec {
            shape: Shape::Keys {
                cardinality: 64,
                dist: Distribution::Uniform,
                dtype,
            },
            size: 32,
            seed: 1,
        };
        let col = spec.generate().unwrap();
        io::write_bin(&path, &col).unwrap();
        io::write_meta(&path, &GenMeta::new(&spec, &col)).unwrap();
        let out = I64Workload::load(&path);
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
        let w = I64Workload::load(&path).expect("no sidecar => assume i64");
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
        std::fs::write(crate::datagen::io::sidecar_path(&path), "{\"not\":\"ours\"}").unwrap();
        let w = I64Workload::load(&path).expect("unparseable sidecar => fall back to i64");
        assert_eq!(w.items(), &[5]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(crate::datagen::io::sidecar_path(&path)).ok();
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
        let w = I64Workload::load(&path).unwrap();
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
        let err = I64Workload::load(&path).unwrap_err();
        assert!(err.to_string().contains("pcap magic"));
        std::fs::remove_file(&path).ok();
    }
}
