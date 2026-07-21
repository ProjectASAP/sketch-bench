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

use aqpbm_datagen::{DType, Distribution, GenSpec, GenValue, Shape, SketchError};

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

    /// The item type the sketch actually ingested.
    ///
    /// Without this field an `i64` run and an `f64` run of the same shape,
    /// size and seed produce **identical** descriptors, so anything that
    /// groups by workload — `--repeats`, `scripts/merge_passes.py`, a
    /// `groupby` over `--raw-csv` — pools two different measurements under
    /// one key and averages them. The dtype is not cosmetic: for the
    /// quantile families it decides whether the library compares integers or
    /// floats, which is the thing being compared.
    ///
    /// Omitted when `i64`, so every record written before this field existed
    /// stays byte-identical and still parses — those runs were all `i64`, so
    /// the default is their true value rather than a guess.
    #[serde(default, skip_serializing_if = "DType::is_i64")]
    pub dtype: DType,
}

impl WorkloadDesc {
    /// Projection of a generator [`Shape`] into the report-facing descriptor,
    /// so JSONL records stay well-formed regardless of shape. Shapes the flat
    /// fields cannot express carry their full spec in `spec`.
    ///
    /// Lives here rather than on `Shape` because it is a question about *this*
    /// type: which of the descriptor's fields can hold a given shape. Keeping
    /// it on `Shape` made the generator reference the report schema, which is
    /// backwards — the generator has no business knowing a report exists, and
    /// that single reference was the only thing preventing `sketch-datagen`
    /// from standing on its own.
    pub fn from_spec(spec: &GenSpec) -> Self {
        let (shape, size, seed) = (&spec.shape, spec.size, spec.seed);
        let (cardinality, zipf_s) = match shape {
            Shape::Keys {
                cardinality, dist, ..
            } => (
                Some(*cardinality),
                match dist {
                    Distribution::Zipf { s } => Some(*s),
                    _ => None,
                },
            ),
            Shape::Categorical { categories, .. } => (Some(categories.len() as u64), None),
            Shape::Monotonic { .. } => (None, None),
        };
        WorkloadDesc {
            shape: shape.report_label().to_string(),
            size,
            cardinality,
            zipf_s,
            source_path: None,
            seed: Some(seed),
            spec: if fits_legacy_desc(shape) {
                None
            } else {
                serde_json::to_value(shape).ok()
            },
            dtype: spec.dtype,
        }
    }
}

/// Whether the flat `cardinality` / `zipf_s` fields fully describe `shape`.
///
/// True only for the two shapes that predate the generator (`keys` drawn
/// uniform or zipf) — those round-trip through the legacy fields exactly, so
/// their records stay byte-identical to what `--workload uniform|zipf` has
/// always emitted. Everything else is lossy there and needs the full spec
/// carried alongside, which is the one condition under which
/// [`WorkloadDesc::spec`] is populated.
fn fits_legacy_desc(shape: &Shape) -> bool {
    matches!(
        shape,
        Shape::Keys {
            dist: Distribution::Uniform | Distribution::Zipf { .. },
            // `dtype` used to be excluded here, because it was part of what
            // makes a shape reproducible and the flat fields could not express
            // it. `WorkloadDesc::dtype` now carries it, so a non-`i64` keys
            // shape round-trips flat like any other and does not need `spec`.
            ..
        }
    )
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
    /// Takes the **1-based repetition index**, not a seed. The seed has to be
    /// derived from the workload's own generation seed, because deriving it
    /// from anything else can silently reproduce the original draw: an
    /// earlier version mixed in `BenchConfig::seed`, which is unrelated to a
    /// `--spec` file's seed, so `spec.seed = 43` with `--seed 42` made
    /// repetition 1 bit-identical to repetition 0 and collapsed the reported
    /// stddev to exactly 0 — the defect this method exists to prevent,
    /// reintroduced silently. Indices start at 1 so a redraw can never
    /// collide with the base draw.
    ///
    /// `None` means this workload has no distribution to redraw from — a file
    /// on disk is one fixed sample. Callers must then run **one** repetition
    /// and report `n = 1`, not N copies of it.
    fn resample(&self, _repetition: usize) -> Option<Self> {
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

// ---------- numeric workloads ----------

// The `NumericItem` trait that used to live here is gone. Its whole job was
// `from_column` — unwrapping the generator's run-time-tagged `Column` into a
// concrete `Vec<T>` and refusing the other variants. The generator is generic
// now, so it hands back a `Vec<T>` directly and there is nothing to unwrap:
// `aqpbm_datagen::GenValue` already carries the `DTYPE` constant this needed,
// and the mismatch check moved into `generate_into`, before any data exists.

/// A numeric workload: the materialised item stream plus its
/// provenance. Construct it from a generator (`uniform` / `zipf`)
/// or from a file (`load`); the source shows up in `desc`, not in
/// the type.
#[derive(Debug, Clone)]
pub struct NumericWorkload<T> {
    items: Vec<T>,
    desc: WorkloadDesc,
    /// The spec this was generated from, when it was generated. Retained so
    /// [`Workload::resample`] can draw again from the same distribution.
    /// `None` for file-backed workloads: a file is one fixed sample.
    spec: Option<GenSpec>,
}

/// The key-shaped workload: every hash-based family (cms, countsketch, hll,
/// elastic, …) ingests these, and the `String`/`Bytes` views derive from it.
pub type I64Workload = NumericWorkload<i64>;

/// The float workload, consumed by the ordered families (kll, dd) whose
/// libraries are `f64`-native.
pub type F64Workload = NumericWorkload<f64>;

impl<T: GenValue> NumericWorkload<T> {
    /// Wrap an already-materialised item stream with its provenance.
    /// `desc.size` is forced to match `items.len()` — a desc that
    /// disagrees with the data it describes would silently corrupt
    /// every throughput denominator downstream.
    pub fn new(items: Vec<T>, mut desc: WorkloadDesc) -> Self {
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
    /// Fails if the spec's dtype does not match `T`: reinterpreting one
    /// numeric encoding as another would yield meaningless items behind a
    /// well-formed report, and — for the ordered families — would hide an
    /// integer-to-float conversion inside a run labelled `f64`.
    pub fn generate(spec: &GenSpec) -> Result<Self, SketchError> {
        let desc = WorkloadDesc::from_spec(spec);
        let mut wk = Self::new(spec.generate::<T>()?, desc);
        wk.spec = Some(spec.clone());
        Ok(wk)
    }

    /// Uniform in `[0, cardinality)`. Convenience over [`Self::generate`].
    pub fn uniform(size: usize, cardinality: u64, seed: u64) -> Self {
        Self::generate(&GenSpec {
            shape: Shape::Keys {
                cardinality,
                dist: Distribution::Uniform,
            },
            size,
            seed,
            dtype: T::DTYPE,
        })
        .expect("uniform keys over a non-zero cardinality always generate")
    }

    /// Zipfian with `s`-parameter (skew exponent) over ranks
    /// `[1, cardinality]`. Convenience over [`Self::generate`].
    pub fn zipf(size: usize, cardinality: u64, s: f64, seed: u64) -> Result<Self, SketchError> {
        Self::generate(&GenSpec {
            shape: Shape::Keys {
                cardinality,
                dist: Distribution::Zipf { s },
            },
            size,
            seed,
            dtype: T::DTYPE,
        })
    }
}

impl NumericWorkload<i64> {
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
    pub fn load(path: &Path) -> Result<Self, SketchError> {
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
            return Err(SketchError::BadParam(format!(
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
                dtype: DType::I64,
            },
        ))
    }
}

impl<T: GenValue> Workload for NumericWorkload<T> {
    type Item = T;
    fn desc(&self) -> WorkloadDesc {
        self.desc.clone()
    }
    fn items(&self) -> &[T] {
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

    /// Regenerate from the retained spec at `spec.seed + repetition`. The
    /// offset is taken from the **spec's own** seed and `repetition >= 1`, so
    /// a redraw can never equal the base draw. Deriving it from anything else
    /// is how this silently breaks: an earlier version mixed in
    /// `BenchConfig::seed`, which is unrelated to a `--spec` file's seed, so
    /// `spec.seed = 43` with `--seed 42` made repetition 1 bit-identical to
    /// repetition 0 and collapsed the reported stddev to exactly 0.
    ///
    /// A spec that generated once cannot fail on a different seed — every
    /// validation in `Shape::build` is seed-independent — so a failure here
    /// would be a bug, and `None` degrades to "cannot vary", which the caller
    /// already handles honestly.
    fn resample(&self, repetition: usize) -> Option<Self> {
        debug_assert!(repetition >= 1, "repetition 0 is the base draw");
        let spec = self.spec.as_ref()?;
        let mut respec = spec.clone();
        respec.seed = spec.seed.wrapping_add(repetition as u64);
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
/// [`aqpbm_datagen::io::read_meta`] error.
fn reject_non_i64_bin(path: &Path) -> Result<(), SketchError> {
    let Ok(Some(meta)) = aqpbm_datagen::io::read_meta(path) else {
        return Ok(());
    };
    if meta.dtype != aqpbm_datagen::DType::I64 {
        return Err(SketchError::BadParam(format!(
            "{}: sidecar declares dtype {}, but the benchmark only consumes i64. \
             Re-generate with `--dtype i64`; reading it as i64 would silently \
             reinterpret the raw bytes and produce meaningless keys.",
            path.display(),
            meta.dtype.as_str(),
        )));
    }
    Ok(())
}

fn load_bin(path: &Path) -> Result<Vec<i64>, SketchError> {
    let bytes = std::fs::read(path).map_err(SketchError::Io)?;
    if bytes.len() % 8 != 0 {
        return Err(SketchError::BadParam(format!(
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

fn load_csv(path: &Path) -> Result<Vec<i64>, SketchError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(SketchError::Io)?;
    let mut items = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(SketchError::Io)?;
        if idx == 0 {
            // Header row.
            continue;
        }
        let field = line.split(',').next().unwrap_or("").trim();
        if field.is_empty() {
            continue;
        }
        let v: i64 = field.parse().map_err(|e| {
            SketchError::BadParam(format!(
                "{}: bad i64 on line {}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        items.push(v);
    }
    Ok(items)
}

fn load_pcap(path: &Path) -> Result<Vec<i64>, SketchError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(SketchError::Io)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(SketchError::Io)?;
    if buf.len() < 24 {
        return Err(SketchError::BadParam(format!(
            "{}: pcap smaller than header",
            path.display()
        )));
    }
    let big_endian = match &buf[..4] {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => false,
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => true,
        _ => {
            return Err(SketchError::BadParam(format!(
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
            return Err(SketchError::BadParam(format!(
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
                dtype: DType::I64,
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
    fn load_generated(dtype: aqpbm_datagen::DType, tag: &str) -> Result<I64Workload, SketchError> {
        use aqpbm_datagen::{io, Distribution, GenMeta, GenSpec, Shape};
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = GenSpec {
            shape: Shape::Keys {
                cardinality: 64,
                dist: Distribution::Uniform,
            },
            size: 32,
            seed: 1,
            dtype,
        };
        // The one shape a run-time dtype takes now: a `match` that picks the
        // type parameter, with every arm one line. No `_` arm, so adding a
        // `DType` variant fails to compile here rather than silently missing
        // a case.
        fn write<T: aqpbm_datagen::GenValue + aqpbm_datagen::FixedWidth>(
            path: &std::path::Path,
            spec: &GenSpec,
        ) {
            let col = spec.generate::<T>().unwrap();
            io::write_bin(path, &col).unwrap();
            io::write_meta(path, &GenMeta::new(spec, &col)).unwrap();
        }
        match dtype {
            aqpbm_datagen::DType::I64 => write::<i64>(&path, &spec),
            aqpbm_datagen::DType::U64 => write::<u64>(&path, &spec),
            aqpbm_datagen::DType::F64 => write::<f64>(&path, &spec),
        }
        let out = I64Workload::load(&path);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(io::sidecar_path(&path)).ok();
        out
    }

    #[test]
    fn bin_with_i64_sidecar_loads() {
        let w = load_generated(aqpbm_datagen::DType::I64, "i64").expect("i64 must load");
        assert_eq!(w.items().len(), 32);
    }

    #[test]
    fn bin_with_non_i64_sidecar_is_rejected() {
        // An f64/u64 stream is byte-indistinguishable from i64, so
        // loading it would silently produce garbage keys rather than
        // fail. The sidecar is the only thing that can catch it.
        for (dtype, tag) in [
            (aqpbm_datagen::DType::F64, "f64"),
            (aqpbm_datagen::DType::U64, "u64"),
        ] {
            let err = load_generated(dtype, tag)
                .expect_err("non-i64 dtype must be rejected, not silently misread");
            let msg = err.to_string();
            assert!(
                msg.contains(tag),
                "error should name the offending dtype: {msg}"
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
        std::fs::write(aqpbm_datagen::io::sidecar_path(&path), "{\"not\":\"ours\"}").unwrap();
        let w = I64Workload::load(&path).expect("unparseable sidecar => fall back to i64");
        assert_eq!(w.items(), &[5]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(aqpbm_datagen::io::sidecar_path(&path)).ok();
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

#[cfg(test)]
mod resample_tests {
    use super::*;
    use aqpbm_datagen::{DType, Distribution, GenSpec, Shape};

    fn spec(seed: u64) -> GenSpec {
        GenSpec {
            shape: Shape::Keys {
                cardinality: 1000,
                dist: Distribution::Zipf { s: 1.1 },
            },
            size: 2000,
            seed,
            dtype: DType::I64,
        }
    }

    /// The property the accuracy pass depends on: no repetition may reproduce
    /// the base draw. A previous version derived the redraw seed from
    /// `BenchConfig::seed`, unrelated to the spec's own seed, so a spec seed
    /// one greater than the CLI seed made repetition 1 identical to
    /// repetition 0 — reported as `accuracy_runs: 2, stddev: 0.0`.
    #[test]
    fn no_repetition_reproduces_the_base_draw() {
        for base_seed in [0u64, 1, 42, 43, u64::MAX] {
            let w = I64Workload::generate(&spec(base_seed)).unwrap();
            for rep in 1..=8usize {
                let r = w.resample(rep).expect("generated workloads resample");
                assert_ne!(
                    r.items(),
                    w.items(),
                    "repetition {rep} reproduced the base draw at seed {base_seed}"
                );
            }
        }
    }

    #[test]
    fn repetitions_differ_from_each_other() {
        let w = I64Workload::generate(&spec(7)).unwrap();
        let a = w.resample(1).unwrap();
        let b = w.resample(2).unwrap();
        assert_ne!(a.items(), b.items());
    }

    #[test]
    fn file_backed_workloads_cannot_resample() {
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_resample_file.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        for v in [1i64, 2, 3, 4] {
            f.write_all(&v.to_le_bytes()).unwrap();
        }
        drop(f);
        let w = I64Workload::load(&path).unwrap();
        assert!(!w.can_resample(), "a file is one fixed sample");
        assert!(w.resample(1).is_none());
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use aqpbm_datagen::{BinSink, Distribution, GenSpec, Shape};

    /// `workload generate` (file sink) and `bench --spec` (memory sink) must
    /// be the same workload, or a run cannot be reproduced from the file it
    /// was supposedly generated into.
    ///
    /// Lives here rather than in `sketch-datagen` because reading a `.bin`
    /// back is `I64Workload::load`. The generator crate can write the format
    /// but not read it, so it cannot check its own round trip — worth fixing,
    /// but not by leaving the assertion unmade.
    #[test]
    fn file_and_memory_sinks_agree() {
        let s = GenSpec {
            shape: Shape::Keys {
                cardinality: 500,
                dist: Distribution::Zipf { s: 1.3 },
            },
            size: 3_000,
            seed: 7,
            dtype: DType::I64,
        };
        let path = std::env::temp_dir().join("sketchlib_sink_agreement.bin");

        let mut file_sink = BinSink::<i64>::create(&path).unwrap();
        s.generate_into(&mut file_sink, 64).unwrap();

        let from_file = I64Workload::load(&path).unwrap();
        let from_memory = I64Workload::generate(&s).unwrap();
        assert_eq!(from_file.items(), from_memory.items());
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(test)]
mod dtype_tests {
    use super::*;
    use aqpbm_datagen::{Distribution, GenSpec, Shape};

    fn keys_spec(dtype: DType) -> GenSpec {
        GenSpec {
            shape: Shape::Keys {
                cardinality: 100,
                dist: Distribution::Uniform,
            },
            size: 500,
            seed: 7,
            dtype,
        }
    }

    /// The dtype axis is only worth having if the two runs are
    /// distinguishable downstream. `--repeats`, `merge_passes.py` and any
    /// `groupby` over the CSV key on the serialised workload, so if these two
    /// descriptors matched, an i64 and an f64 measurement would be averaged
    /// together under one row.
    #[test]
    fn i64_and_f64_descriptors_are_distinguishable() {
        let a = I64Workload::generate(&keys_spec(DType::I64)).unwrap();
        let b = F64Workload::generate(&keys_spec(DType::F64)).unwrap();
        let (ja, jb) = (
            serde_json::to_string(&a.desc()).unwrap(),
            serde_json::to_string(&b.desc()).unwrap(),
        );
        assert_ne!(ja, jb, "i64 and f64 workloads must not share a group key");
        assert!(jb.contains(r#""dtype":"f64""#), "{jb}");
    }

    /// Every record ever written was i64, so the field is omitted at that
    /// value: old files stay byte-identical and new i64 runs still compare
    /// equal to them.
    #[test]
    fn an_i64_descriptor_keeps_the_bytes_it_had_before_the_field_existed() {
        let wk = I64Workload::generate(&keys_spec(DType::I64)).unwrap();
        let json = serde_json::to_string(&wk.desc()).unwrap();
        assert!(
            !json.contains("dtype"),
            "i64 must not emit the field: {json}"
        );
    }

    #[test]
    fn a_descriptor_without_dtype_reads_back_as_i64() {
        let old = r#"{"shape":"uniform","size":500,"cardinality":100,"seed":7}"#;
        let desc: WorkloadDesc = serde_json::from_str(old).unwrap();
        assert_eq!(desc.dtype, DType::I64);
    }

    /// The one thing this axis must never do: quietly widen integers into the
    /// float path. That would put an `as f64` back on the insert loop while
    /// the report claims the workload was f64 — the measurement error the
    /// dtype axis exists to expose.
    #[test]
    fn a_float_workload_refuses_an_integer_spec() {
        let err = F64Workload::generate(&keys_spec(DType::I64))
            .unwrap_err()
            .to_string();
        assert!(err.contains("f64") && err.contains("i64"), "{err}");
        // And the converse, so neither direction converts.
        assert!(I64Workload::generate(&keys_spec(DType::F64)).is_err());
    }
}
