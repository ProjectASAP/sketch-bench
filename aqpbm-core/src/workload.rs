//! Synthetic workload generators + adapters for file-backed test data.
//! See `docs/DESIGN.md` §4.3.
//!
//! One type per *item* type, not per source: provenance is data, not a type
//! parameter, so it lives in `description` and the source picks a constructor.

use serde::{Deserialize, Serialize};
use std::path::Path;

use aqpbm_datagen::{Distribution, GenSpec, GenValue, Shape, SketchError};

/// Human-friendly description of a workload — serialised into
/// every report so a JSONL record can be re-run without
/// external metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadDescription {
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
    /// Full `datagen` spec, when the flat fields above cannot express the shape
    /// (categorical weights, timestamp gap distributions, …). Absent for
    /// `uniform` / `zipf` / `file`, whose flat fields already round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
}

impl WorkloadDescription {
    /// Projection of a generator [`Shape`] into the report-facing descriptor.
    /// Lives here, not on `Shape`, because it asks which of *this* type's
    /// fields can hold a shape — on `Shape` the generator would cite the schema.
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
        WorkloadDescription {
            shape: shape.report_label().to_string(),
            size,
            cardinality,
            zipf_s,
            source_path: None,
            seed: Some(seed),
            spec: if fits_legacy_description(spec) {
                None
            } else {
                serde_json::to_value(spec).ok()
            },
        }
    }
}

/// Whether the flat `cardinality` / `zipf_s` fields fully describe `shape`.
/// True only for `keys` drawn uniform or zipf; everything else is lossy there
/// and is the one condition under which [`WorkloadDescription::spec`] is set.
fn fits_legacy_description(spec: &GenSpec) -> bool {
    // `string` opts change the keys without changing the shape, so a spec
    // carrying them cannot round-trip through the flat fields either: two
    // runs at different key lengths would share a descriptor and be pooled.
    spec.string.is_none()
        && matches!(
            spec.shape,
            Shape::Keys {
                dist: Distribution::Uniform | Distribution::Zipf { .. },
                ..
            }
        )
}

/// The abstract contract for a workload a `BenchRunner` can
/// consume. An implementation produces an ordered `Vec<Item>`
/// plus an optional query stream.
pub trait Workload: Sized {
    type Item: Clone;
    fn description(&self) -> WorkloadDescription;
    fn items(&self) -> &[Self::Item];

    /// An **independent draw** from the same distribution — error is
    /// deterministic given (data, parameters), so repeats over one fixed workload
    /// fabricate spread. 1-based index; seed derives from the workload's own.
    fn resample(&self, _repetition: usize) -> Option<Self> {
        None
    }

    /// Whether [`Self::resample`] can produce anything, without paying for a
    /// generation to find out — the runner needs this *before* choosing how
    /// many repetitions to run.
    fn can_resample(&self) -> bool {
        false
    }
}

// ---------- numeric workloads ----------

/// A numeric workload: the materialised item stream plus its provenance.
/// Construct from a generator (`uniform` / `zipf`) or a file (`load`); the
/// source shows up in `description`, not in the type.
#[derive(Debug, Clone)]
pub struct NumericWorkload<T> {
    items: Vec<T>,
    description: WorkloadDescription,
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
    /// `description.size` is forced to `items.len()`: a description disagreeing
    /// with its data would corrupt every throughput denominator downstream.
    pub fn new(items: Vec<T>, mut description: WorkloadDescription) -> Self {
        // `load` passes 0 as a placeholder, not knowing the count until it has
        // read the file. Any other value asserts what was produced — otherwise a
        // short generator is silently relabelled into a smaller workload.
        debug_assert!(
            description.size == 0 || description.size == items.len(),
            "workload description claims {} items but carries {}",
            description.size,
            items.len(),
        );
        description.size = items.len();
        Self {
            items,
            description,
            spec: None,
        }
    }

    /// Generate in-process from a [`GenSpec`] — the one generator in the tool.
    /// `workload generate` runs the same spec through a file sink and this
    /// through a memory sink, so on-disk shapes are reachable here.
    pub fn generate(spec: &GenSpec) -> Result<Self, SketchError> {
        let description = WorkloadDescription::from_spec(spec);
        let mut wk = Self::new(spec.generate::<T>()?, description);
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
            string: None,
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
            string: None,
        })
    }
}

impl NumericWorkload<i64> {
    /// Load from a file, format from the extension: `.bin` (and anything else)
    /// is a little-endian `int64` stream; `.pcap` takes each IPv4 source address
    /// as big-endian `u32`; `.csv` parses column 0 below a header row.
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
            WorkloadDescription {
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

impl<T: GenValue> Workload for NumericWorkload<T> {
    type Item = T;
    fn description(&self) -> WorkloadDescription {
        self.description.clone()
    }
    fn items(&self) -> &[T] {
        &self.items
    }

    /// True exactly when a spec was retained: generated workloads can redraw,
    /// file-backed ones cannot.
    fn can_resample(&self) -> bool {
        self.spec.is_some()
    }

    /// Regenerate at `spec.seed + repetition` — the offset must come from the
    /// **spec's own** seed. Every validation in `Shape::build` is
    /// seed-independent, so a spec that generated once cannot fail here.
    fn resample(&self, repetition: usize) -> Option<Self> {
        debug_assert!(repetition >= 1, "repetition 0 is the base draw");
        let spec = self.spec.as_ref()?;
        let mut respec = spec.clone();
        respec.seed = spec.seed.wrapping_add(repetition as u64);
        Self::generate(&respec).ok()
    }
}

/// Reject a `.bin` whose sidecar declares an unreadable dtype. The stream is
/// header-less, so an `f64` file is byte-indistinguishable from `i64` — `0.093`
/// reads back as `4591388162153532928`. No sidecar means "assume i64".
fn reject_non_i64_bin(path: &Path) -> Result<(), SketchError> {
    let Ok(Some(meta)) = aqpbm_datagen::io::read_meta(path) else {
        return Ok(());
    };
    if meta.dtype != "i64" {
        return Err(SketchError::BadParam(format!(
            "{}: sidecar declares dtype {}, but the benchmark only consumes i64. \
             Re-generate with `--dtype i64`; reading it as i64 would silently \
             reinterpret the raw bytes and produce meaningless keys.",
            path.display(),
            meta.dtype,
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

// ---------- string / bytes workloads ----------

/// A `String` workload, either **generated** (varying length, configurable
/// alphabet) or **derived** via [`Self::from_i64`] (1-7 decimal digits). They
/// measure different things, so pooling them averages a real one with a fake.
pub type StringWorkload = NumericWorkload<String>;

impl NumericWorkload<String> {
    /// Decimal-format an `i64` workload. Not a string workload in any meaningful
    /// sense — hash cost and length are what one exists to vary, and here both
    /// follow from the integer. No `spec`, so `resample` yields `None`.
    pub fn from_i64(inner: &I64Workload) -> Self {
        Self {
            items: inner.items().iter().map(|v| v.to_string()).collect(),
            description: inner.description(),
            spec: None,
        }
    }
}

/// Same, but `Vec<u8>` for impls that want `&[u8]`. Not a [`NumericWorkload`]:
/// `Vec<u8>` is not a `GenValue`, so these rows take the bytes of whichever
/// string workload is in play.
#[derive(Debug, Clone)]
pub struct BytesWorkload {
    items: Vec<Vec<u8>>,
    description: WorkloadDescription,
}

impl BytesWorkload {
    pub fn from_i64(inner: &I64Workload) -> Self {
        Self {
            items: inner
                .items()
                .iter()
                .map(|v| v.to_string().into_bytes())
                .collect(),
            description: inner.description(),
        }
    }

    /// Bytes of an existing string workload, generated or derived. Carries
    /// its `description`, so a run over real strings stays distinguishable from one
    /// over decimal-formatted integers.
    pub fn from_strings(inner: &StringWorkload) -> Self {
        Self {
            items: inner
                .items()
                .iter()
                .map(|s| s.clone().into_bytes())
                .collect(),
            description: inner.description(),
        }
    }
}

impl Workload for BytesWorkload {
    type Item = Vec<u8>;
    fn description(&self) -> WorkloadDescription {
        self.description.clone()
    }
    fn items(&self) -> &[Vec<u8>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Key length is the dominant cost on a hashing insert path, so two runs
    /// at different lengths are two measurements. They must not share a
    /// descriptor, or anything grouping by workload averages them together.
    #[test]
    fn string_options_reach_the_descriptor() {
        let base = GenSpec {
            shape: Shape::Keys {
                cardinality: 64,
                dist: Distribution::Uniform,
            },
            size: 32,
            seed: 1,
            string: None,
        };
        let with_opts = |min_len, max_len| GenSpec {
            string: Some(aqpbm_datagen::StringOpts {
                alphabet: "ab".to_string(),
                min_len,
                max_len,
            }),
            ..base.clone()
        };

        // Defaults keep the descriptor a record written before the flags
        // existed would have had.
        let plain = WorkloadDescription::from_spec(&base);
        assert!(plain.spec.is_none(), "{plain:?}");

        let short = WorkloadDescription::from_spec(&with_opts(4, 4));
        let long = WorkloadDescription::from_spec(&with_opts(16, 16));
        assert!(short.spec.is_some());
        assert_ne!(
            serde_json::to_string(&short).unwrap(),
            serde_json::to_string(&long).unwrap(),
            "two key lengths must not share a group key"
        );
    }

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
        assert_eq!(s.description().shape, "uniform");
    }

    #[test]
    fn placeholder_description_size_is_filled_in() {
        // A description that disagrees with the data would silently skew every
        // throughput denominator; `new` is the one place that can catch
        // it, so it always wins over the caller's claim.
        let w = I64Workload::new(
            vec![1, 2, 3],
            WorkloadDescription {
                shape: "custom".into(),
                size: 0, // placeholder, as `load` passes
                cardinality: None,
                zipf_s: None,
                source_path: None,
                seed: None,
                spec: None,
            },
        );
        assert_eq!(w.description().size, 3);
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
        assert_eq!(w.description().shape, "file");
        assert_eq!(w.description().size, 4);
        std::fs::remove_file(&path).ok();
    }

    /// Write a `.bin` + sidecar at item type `T` and try to load it as i64.
    /// The type parameter is the only thing naming an encoding — there is no
    /// tag to pass, and no match to keep exhaustive.
    fn load_generated<T: aqpbm_datagen::GenValue + aqpbm_datagen::FixedWidth>(
        tag: &str,
    ) -> Result<I64Workload, SketchError> {
        use aqpbm_datagen::{io, Distribution, GenMeta, GenSpec, Shape};
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = GenSpec {
            shape: Shape::Keys {
                cardinality: 64,
                dist: Distribution::Uniform,
            },
            size: 32,
            seed: 1,
            string: None,
        };
        let col = spec.generate::<T>().unwrap();
        io::write_bin(&path, &col).unwrap();
        io::write_meta(&path, &GenMeta::new(&spec, &col)).unwrap();
        let out = I64Workload::load(&path);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(io::sidecar_path(&path)).ok();
        out
    }

    #[test]
    fn bin_with_i64_sidecar_loads() {
        let w = load_generated::<i64>("i64").expect("i64 must load");
        assert_eq!(w.items().len(), 32);
    }

    #[test]
    fn bin_with_non_i64_sidecar_is_rejected() {
        // An f64/u64 stream is byte-indistinguishable from i64, so
        // loading it would silently produce garbage keys rather than
        // fail. The sidecar is the only thing that can catch it.
        for (err, tag) in [
            (load_generated::<f64>("f64"), "f64"),
            (load_generated::<u64>("u64"), "u64"),
        ] {
            let err = err.expect_err("a non-i64 stream must be rejected, not silently misread");
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
    use aqpbm_datagen::{Distribution, GenSpec, Shape};

    fn spec(seed: u64) -> GenSpec {
        GenSpec {
            shape: Shape::Keys {
                cardinality: 1000,
                dist: Distribution::Zipf { s: 1.1 },
            },
            size: 2000,
            seed,
            string: None,
        }
    }

    /// The property the accuracy pass depends on: no repetition may reproduce
    /// the base draw. Deriving the redraw seed from anything but the spec's own
    /// seed can collide, and a collision reports as `stddev: 0.0`.
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

    /// `workload generate` (file sink) and `bench --spec` (memory sink) must be
    /// the same workload, or a run cannot be reproduced from its own file. Lives
    /// here because reading a `.bin` back is `I64Workload::load`.
    #[test]
    fn file_and_memory_sinks_agree() {
        let s = GenSpec {
            shape: Shape::Keys {
                cardinality: 500,
                dist: Distribution::Zipf { s: 1.3 },
            },
            size: 3_000,
            seed: 7,
            string: None,
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
