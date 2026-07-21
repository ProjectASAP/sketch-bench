//! Synthetic data-generation toolkit.
//!
//! A small, extensible library for producing benchmark workloads as
//! raw little-endian `.bin` files (consumed by `sketchlib bench
//! --input`) alongside a self-describing `.meta.json` sidecar.
//!
//! The design is intentionally decoupled from the benchmark: generators
//! write files, the benchmark reads them through the existing
//! [`crate::workload::FileI64`] loader, so no dispatch machinery has to
//! know a new distribution exists. See `docs/DESIGN.md` §4.3.
//!
//! ## Extending
//!
//! Distribution (*how* values spread) and structure (*what* they mean)
//! are orthogonal axes:
//!
//! * New distribution → add a [`dist::Distribution`] variant plus the
//!   arm(s) in whichever realization it supports (`key_sampler` /
//!   `weights` / `gap_sampler`). Every structure picks it up for free.
//! * New structure → add a `*Gen` struct, a [`shape::Generator`]
//!   variant, and a variant + build arm to [`shape::Shape`].
//! * New physical type → add a [`DType`] variant and a [`Column`] arm.
//!
//! ## Reproducibility
//!
//! Every generator is a pure function of `(spec, seed, n)`: it takes a
//! freshly seeded [`Xoshiro256PlusPlus`] and holds any running state
//! (e.g. a timestamp accumulator) as a call-local, so the same inputs
//! always yield byte-identical output.

pub mod dist;
pub mod io;
pub mod shape;
pub mod sink;
pub mod stats;

use std::io::Write;
use std::path::Path;

use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;

pub use dist::Distribution;
pub use shape::{Generator, Shape, TimeUnit};
pub use sink::{FileSink, MemorySink, Sink, WriterSink};
pub use stats::{BasicStats, StatsAcc};

/// Values produced per [`Sink::accept`] call by [`GenSpec::generate_into`].
///
/// Bounds a streaming sink's memory (64Ki × 8B = 512KiB per chunk) while
/// staying large enough that the per-chunk dispatch is noise next to the
/// per-value sampling. It is a transport detail only: output is
/// byte-identical at any chunk size.
pub const DEFAULT_CHUNK: usize = 1 << 16;

/// Schema version of the `.meta.json` sidecar. Bumped to 2 when the
/// `shape` representation was refactored into orthogonal
/// structure/`Distribution` axes; v1 sidecars (flattened `uniform`/
/// `zipf`/`monotonic_timestamp`/`skewed_categorical` shapes) no longer
/// deserialize.
pub const GEN_META_SCHEMA_VERSION: u32 = 2;

/// Physical output type of a generated column. The `.bin` stream is a
/// raw little-endian sequence of this type; the logical dtype is
/// recorded in the sidecar (the `.bin` itself is header-less).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    /// Signed 64-bit — the only type the benchmark consumes today.
    #[default]
    I64,
    /// Unsigned 64-bit.
    U64,
    /// IEEE-754 double.
    F64,
}

impl DType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DType::I64 => "i64",
            DType::U64 => "u64",
            DType::F64 => "f64",
        }
    }
}

/// A fully-materialized, typed column of generated values.
///
/// The generator eagerly builds the whole column (matching the
/// benchmark's eager `Vec<Item>` consumption); there is no streaming
/// downstream to preserve.
#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    I64(Vec<i64>),
    U64(Vec<u64>),
    F64(Vec<f64>),
}

impl Column {
    pub fn dtype(&self) -> DType {
        match self {
            Column::I64(_) => DType::I64,
            Column::U64(_) => DType::U64,
            Column::F64(_) => DType::F64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Column::I64(v) => v.len(),
            Column::U64(v) => v.len(),
            Column::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write as a raw little-endian stream with no header, matching the
    /// `input/benchmark_data_*.bin` layout for the `i64` case.
    pub fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        match self {
            Column::I64(v) => {
                for x in v {
                    w.write_all(&x.to_le_bytes())?;
                }
            }
            Column::U64(v) => {
                for x in v {
                    w.write_all(&x.to_le_bytes())?;
                }
            }
            Column::F64(v) => {
                for x in v {
                    w.write_all(&x.to_le_bytes())?;
                }
            }
        }
        Ok(())
    }

    /// Descriptive summary for the sidecar / `describe`.
    pub fn stats(&self) -> BasicStats {
        let mut acc = StatsAcc::new();
        self.accumulate_stats(&mut acc);
        acc.finish()
    }

    /// Fold this chunk into a running summary. Used by the chunked
    /// driver, where no single slice holds the whole column.
    pub(crate) fn accumulate_stats(&self, acc: &mut StatsAcc) {
        match self {
            Column::I64(v) => acc.push_slice(v, |x| *x as f64),
            Column::U64(v) => acc.push_slice(v, |x| *x as f64),
            Column::F64(v) => acc.push_slice(v, |x| *x),
        }
    }

    /// Take the values as `i64`, or fail if this column is another
    /// dtype. The benchmark only consumes `i64`; reinterpreting a
    /// `u64`/`f64` bit pattern as `i64` would produce meaningless keys
    /// and a plausible-looking report, so this refuses rather than
    /// casts (same contract as the `.bin` sidecar guard in
    /// `crate::workload`).
    pub fn into_i64(self) -> Result<Vec<i64>, SketchCoreError> {
        match self {
            Column::I64(v) => Ok(v),
            other => Err(SketchCoreError::BadParam(format!(
                "workload must be i64, got {}; re-generate with dtype i64",
                other.dtype().as_str()
            ))),
        }
    }
}

fn default_seed() -> u64 {
    42
}

/// A complete generation request: which [`Shape`], how many rows, and
/// the seed. This is what a `--spec` YAML/JSON file deserializes into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenSpec {
    #[serde(flatten)]
    pub shape: Shape,
    pub size: usize,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

impl GenSpec {
    /// Load a spec from a `.yaml`/`.yml` (serde_yaml) or otherwise JSON
    /// file. The shape's tagged fields are flattened alongside
    /// `size`/`seed`, e.g.:
    ///
    /// ```yaml
    /// shape: keys
    /// cardinality: 100000
    /// dist: { kind: zipf, s: 1.1 }
    /// dtype: i64
    /// size: 1000000
    /// seed: 42
    /// ```
    pub fn from_path(path: &Path) -> Result<Self, SketchCoreError> {
        let text = std::fs::read_to_string(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("yaml") | Some("yml") => serde_yaml::from_str(&text)
                .map_err(|e| SketchCoreError::BadParam(format!("spec yaml: {e}"))),
            _ => serde_json::from_str(&text)
                .map_err(|e| SketchCoreError::BadParam(format!("spec json: {e}"))),
        }
    }

    /// Generate the whole column into memory.
    ///
    /// Convenience wrapper over [`Self::generate_into`] with a
    /// [`MemorySink`], for callers that want the values resident (the
    /// benchmark runner replays one slice per measured run) and know
    /// the dataset fits.
    pub fn generate(&self) -> Result<Column, SketchCoreError> {
        let mut sink = MemorySink::new();
        self.generate_into(&mut sink, DEFAULT_CHUNK)?;
        Ok(sink.into_column().unwrap_or(Column::I64(Vec::new())))
    }

    /// Generate into `sink`, `chunk` values at a time, and return the
    /// provenance record for what was written.
    ///
    /// The chunking is what decouples dataset size from memory: a
    /// [`FileSink`] streams a dataset far larger than RAM, while a
    /// [`MemorySink`] reassembles the same bytes. `chunk` therefore
    /// affects only peak memory and never the output — see
    /// `chunk_size_does_not_change_output`.
    pub fn generate_into<S: Sink>(
        &self,
        sink: &mut S,
        chunk: usize,
    ) -> Result<GenMeta, SketchCoreError> {
        use rand::SeedableRng;
        if chunk == 0 {
            return Err(SketchCoreError::BadParam("chunk size must be > 0".into()));
        }
        let mut generator = self.shape.build()?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(self.seed);
        let mut stats = StatsAcc::new();

        let mut remaining = self.size;
        while remaining > 0 {
            let n = remaining.min(chunk);
            let col = generator.generate(n, &mut rng)?;
            col.accumulate_stats(&mut stats);
            sink.accept(&col)?;
            remaining -= n;
        }
        sink.flush()?;

        Ok(GenMeta::from_parts(self, self.shape.dtype(), stats.finish()))
    }
}

/// Provenance written to the `foo.bin.meta.json` sidecar. Carries the
/// full [`Shape`] (round-trips exactly) so `describe` can recover how a
/// file was made even though the `.bin` itself is header-less.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenMeta {
    pub schema_version: u32,
    pub generator_version: String,
    pub dtype: DType,
    pub count: usize,
    pub seed: u64,
    pub shape: Shape,
    pub stats: BasicStats,
}

impl GenMeta {
    /// Assemble the sidecar record for a generated column.
    pub fn new(spec: &GenSpec, col: &Column) -> Self {
        Self::from_parts(spec, col.dtype(), col.stats())
    }

    /// Assemble the sidecar record from a streamed generation, where no
    /// single `Column` ever existed to describe.
    pub fn from_parts(spec: &GenSpec, dtype: DType, stats: BasicStats) -> Self {
        GenMeta {
            schema_version: GEN_META_SCHEMA_VERSION,
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            dtype,
            count: stats.count,
            seed: spec.seed,
            shape: spec.shape.clone(),
            stats,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn spec(shape: Shape, size: usize, seed: u64) -> GenSpec {
        GenSpec { shape, size, seed }
    }

    fn keys(cardinality: u64, dist: Distribution, dtype: DType) -> Shape {
        Shape::Keys {
            cardinality,
            dist,
            dtype,
        }
    }

    #[test]
    fn uniform_is_reproducible_and_in_range() {
        let s = spec(keys(1000, Distribution::Uniform, DType::I64), 500, 42);
        let a = s.generate().unwrap();
        let b = s.generate().unwrap();
        assert_eq!(a, b, "same spec+seed must be byte-identical");
        match a {
            Column::I64(v) => {
                assert_eq!(v.len(), 500);
                assert!(v.iter().all(|x| (0..1000).contains(x)));
            }
            _ => panic!("expected i64 column"),
        }
    }

    #[test]
    fn zipf_ranks_in_expected_range() {
        let s = spec(keys(100, Distribution::Zipf { s: 1.1 }, DType::I64), 1000, 7);
        match s.generate().unwrap() {
            Column::I64(v) => assert!(v.iter().all(|x| (1..=100).contains(x))),
            _ => panic!("expected i64 column"),
        }
    }

    #[test]
    fn dtype_selects_physical_width() {
        let base = |dtype| {
            spec(keys(256, Distribution::Uniform, dtype), 64, 1)
                .generate()
                .unwrap()
        };
        assert!(matches!(base(DType::I64), Column::I64(_)));
        assert!(matches!(base(DType::U64), Column::U64(_)));
        assert!(matches!(base(DType::F64), Column::F64(_)));
        // 64 values × 8 bytes each, every dtype.
        for c in [base(DType::I64), base(DType::U64), base(DType::F64)] {
            let mut buf = Vec::new();
            c.write_le(&mut buf).unwrap();
            assert_eq!(buf.len(), 64 * 8);
        }
    }

    #[test]
    fn uniform_cardinality_means_distinct_count_for_every_dtype() {
        // `cardinality` must not silently change meaning with dtype:
        // a continuous f64 range would yield ~n distinct values instead
        // of `cardinality`, quietly invalidating the one parameter a
        // sketch benchmark cares most about.
        let card = 100u64;
        let col = |dtype| {
            spec(keys(card, Distribution::Uniform, dtype), 10_000, 42)
                .generate()
                .unwrap()
        };
        let f64s = match col(DType::F64) {
            Column::F64(v) => v,
            _ => panic!("expected f64 column"),
        };
        let distinct = f64s
            .iter()
            .map(|x| x.to_bits())
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(distinct, card as usize, "f64 must draw from `cardinality` keys");
        assert!(
            f64s.iter().all(|x| x.fract() == 0.0 && (0.0..card as f64).contains(x)),
            "f64 values must be whole numbers in [0, cardinality)"
        );
    }

    #[test]
    fn dtype_changes_encoding_not_logical_values() {
        // Holding shape+size+seed fixed, every dtype must produce the
        // same logical sequence — that is what makes dtype a controlled
        // variable when comparing benchmark runs.
        let s = |dtype| {
            spec(keys(500, Distribution::Uniform, dtype), 1_000, 7)
                .generate()
                .unwrap()
        };
        let (i, u, f) = (s(DType::I64), s(DType::U64), s(DType::F64));
        match (i, u, f) {
            (Column::I64(i), Column::U64(u), Column::F64(f)) => {
                assert!(i.iter().zip(&u).all(|(a, b)| *a as u64 == *b));
                assert!(i.iter().zip(&f).all(|(a, b)| *a as f64 == *b));
            }
            _ => panic!("unexpected column types"),
        }
    }

    #[test]
    fn f64_cardinality_past_2p53_is_rejected() {
        // Beyond 2^53 the `as f64` cast rounds, so the key space would
        // silently differ from the i64 run it is meant to mirror.
        let build = |dtype, cardinality| keys(cardinality, Distribution::Uniform, dtype).build();
        assert!(build(DType::F64, (1u64 << 53) + 1).is_err());
        assert!(build(DType::F64, 1u64 << 53).is_ok(), "the limit itself is exact");
        assert!(build(DType::I64, u64::MAX).is_ok(), "i64 is unaffected");
        assert!(
            keys((1u64 << 53) + 1, Distribution::Zipf { s: 1.1 }, DType::F64)
                .build()
                .is_err(),
            "zipf shares the limit"
        );
    }

    #[test]
    fn zero_cardinality_is_rejected() {
        assert!(keys(0, Distribution::Uniform, DType::I64).build().is_err());
    }

    #[test]
    fn keys_reject_non_range_distribution() {
        // Geometric/poisson/explicit have no meaning as a key sampler.
        assert!(keys(100, Distribution::Poisson { lambda: 3.0 }, DType::I64)
            .build()
            .is_err());
    }

    #[test]
    fn meta_round_trips_through_json() {
        let s = spec(keys(50, Distribution::Zipf { s: 1.2 }, DType::U64), 100, 9);
        let col = s.generate().unwrap();
        let meta = GenMeta::new(&s, &col);
        let json = serde_json::to_string(&meta).unwrap();
        let back: GenMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shape, s.shape);
        assert_eq!(back.dtype, DType::U64);
        assert_eq!(back.count, 100);
    }

    #[test]
    fn genspec_deserializes_flattened_shape() {
        let json =
            r#"{"shape":"keys","cardinality":1000,"dist":{"kind":"zipf","s":1.1},"size":10,"seed":3}"#;
        let spec: GenSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.size, 10);
        assert_eq!(spec.seed, 3);
        assert!(matches!(
            spec.shape,
            Shape::Keys {
                cardinality: 1000,
                dist: Distribution::Zipf { .. },
                ..
            }
        ));
    }

    #[test]
    fn dist_defaults_to_uniform_when_omitted() {
        let spec: GenSpec =
            serde_json::from_str(r#"{"shape":"keys","cardinality":8,"size":4}"#).unwrap();
        assert!(matches!(
            spec.shape,
            Shape::Keys {
                dist: Distribution::Uniform,
                ..
            }
        ));
    }

    #[test]
    fn timestamp_is_monotonic_and_starts_at_start() {
        let s = spec(
            Shape::Monotonic {
                start: 1000,
                unit: TimeUnit::Nanos,
                gap: Distribution::Geometric { p: 0.01 },
                min_gap: 1,
                dtype: DType::I64,
            },
            5000,
            42,
        );
        match s.generate().unwrap() {
            Column::I64(v) => {
                assert_eq!(v[0], 1000, "first value must equal start");
                assert!(
                    v.windows(2).all(|w| w[1] > w[0]),
                    "min_gap=1 must be strictly increasing"
                );
            }
            _ => panic!("expected i64 column"),
        }
    }

    #[test]
    fn timestamp_min_gap_zero_allows_duplicates() {
        let s = spec(
            Shape::Monotonic {
                start: 0,
                unit: TimeUnit::Secs,
                gap: Distribution::Constant { value: 0 },
                min_gap: 0,
                dtype: DType::U64,
            },
            10,
            1,
        );
        match s.generate().unwrap() {
            Column::U64(v) => assert!(v.iter().all(|&x| x == 0), "constant-0 gap stays flat"),
            _ => panic!("expected u64 column"),
        }
    }

    #[test]
    fn timestamp_overflow_is_an_error() {
        let s = spec(
            Shape::Monotonic {
                start: i64::MAX - 5,
                unit: TimeUnit::Nanos,
                gap: Distribution::Constant { value: 100 },
                min_gap: 1,
                dtype: DType::I64,
            },
            100,
            1,
        );
        assert!(s.generate().is_err(), "accumulation past i64::MAX must error");
    }

    #[test]
    fn categorical_stays_in_domain_and_respects_skew() {
        let categories: Vec<i64> = (0..40).collect();
        let s = spec(
            Shape::Categorical {
                categories: categories.clone(),
                dist: Distribution::Zipf { s: 1.3 },
            },
            50_000,
            42,
        );
        match s.generate().unwrap() {
            Column::I64(v) => {
                assert!(v.iter().all(|x| categories.contains(x)), "ids in domain");
                let mut counts = [0usize; 40];
                for &x in &v {
                    counts[x as usize] += 1;
                }
                assert!(
                    counts[0] > counts[39],
                    "zipf weights make category 0 heavier than the last"
                );
            }
            _ => panic!("expected i64 column"),
        }
    }

    #[test]
    fn categorical_explicit_length_mismatch_errors() {
        let err = Shape::Categorical {
            categories: vec![1, 2, 3],
            dist: Distribution::Explicit {
                weights: vec![1.0, 2.0],
            },
        }
        .build();
        assert!(err.is_err());
    }

    /// Chunking is a property of the *transport*, never of the data.
    /// If it were not, the file sink and the in-memory benchmark path
    /// would silently disagree about what "the same workload" means.
    #[test]
    fn chunk_size_does_not_change_output() {
        let shapes = [
            keys(1000, Distribution::Zipf { s: 1.1 }, DType::I64),
            Shape::Categorical {
                categories: (0..16).collect(),
                dist: Distribution::Zipf { s: 1.2 },
            },
            // The monotonic accumulator is the one stateful generator:
            // a chunk boundary must not restart the series.
            Shape::Monotonic {
                start: 1_700_000_000_000,
                unit: TimeUnit::Millis,
                gap: Distribution::Exponential { lambda: 0.5 },
                min_gap: 1,
                dtype: DType::I64,
            },
        ];
        for shape in shapes {
            let s = spec(shape.clone(), 5_000, 42);
            let one_shot = {
                let mut sink = MemorySink::new();
                s.generate_into(&mut sink, 1 << 20).unwrap();
                sink.into_column().unwrap()
            };
            for chunk in [1usize, 7, 512, 4999, 5000] {
                let mut sink = MemorySink::new();
                let meta = s.generate_into(&mut sink, chunk).unwrap();
                assert_eq!(
                    sink.into_column().unwrap(),
                    one_shot,
                    "chunk={chunk} changed the values for {shape:?}"
                );
                assert_eq!(meta.count, 5_000, "chunk={chunk} lost rows");
                assert_eq!(meta.stats, one_shot.stats(), "chunk={chunk} skewed the summary");
            }
        }
    }

    #[test]
    fn file_and_memory_sinks_agree() {
        // `workload generate` (file sink) and `bench --spec` (memory
        // sink) must be the same workload, or a run cannot be
        // reproduced from the file it was supposedly generated into.
        let s = spec(keys(500, Distribution::Zipf { s: 1.3 }, DType::I64), 3_000, 7);
        let path = std::env::temp_dir().join("sketchlib_sink_agreement.bin");

        let mut file_sink = crate::datagen::FileSink::create(&path).unwrap();
        s.generate_into(&mut file_sink, 64).unwrap();

        let from_file = crate::workload::I64Workload::load(&path).unwrap();
        let from_memory = crate::workload::I64Workload::generate(&s).unwrap();
        use crate::workload::Workload as _;
        assert_eq!(from_file.items(), from_memory.items());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn bin_write_matches_le_layout() {
        // i64 column bytes equal a hand-rolled LE encoding.
        let col = Column::I64(vec![1, -2, 3]);
        let mut buf = Vec::new();
        col.write_le(&mut buf).unwrap();
        let mut expected = Vec::new();
        for v in [1i64, -2, 3] {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(buf, expected);
        let _ = Xoshiro256PlusPlus::seed_from_u64(0); // keep import used across cfgs
    }
}
