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
//! * New distribution → add a `struct` implementing [`ColumnGenerator`],
//!   a variant to [`shape::Shape`], and one arm to `Shape::build`.
//! * New physical type → add a [`DType`] variant and a [`Column`] arm.
//!
//! ## Reproducibility
//!
//! Every generator is a pure function of `(spec, seed, n)`: it takes a
//! freshly seeded [`Xoshiro256PlusPlus`] and holds any running state
//! (e.g. a timestamp accumulator) as a call-local, so the same inputs
//! always yield byte-identical output.

pub mod gap;
pub mod io;
pub mod shape;
pub mod stats;
pub mod weights;

use std::io::Write;
use std::path::Path;

use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;

pub use gap::GapDist;
pub use shape::{Shape, TimeUnit};
pub use stats::BasicStats;
pub use weights::WeightSpec;

/// Schema version of the `.meta.json` sidecar.
pub const GEN_META_SCHEMA_VERSION: u32 = 1;

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
        match self {
            Column::I64(v) => BasicStats::summarize(v, |x| *x as f64),
            Column::U64(v) => BasicStats::summarize(v, |x| *x as f64),
            Column::F64(v) => BasicStats::summarize(v, |x| *x),
        }
    }
}

/// The abstract generator contract. Implementors own their parameters;
/// `generate` is a pure function of `(self, n, rng)`.
pub trait ColumnGenerator {
    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus)
        -> Result<Column, SketchCoreError>;

    /// The physical type `generate` will emit.
    fn dtype(&self) -> DType;
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
    /// shape: zipf
    /// cardinality: 100000
    /// s: 1.1
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

    /// Build the generator and produce the column from a freshly seeded
    /// RNG. This is the single entry point callers should use.
    pub fn generate(&self) -> Result<Column, SketchCoreError> {
        use rand::SeedableRng;
        let generator = self.shape.build()?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(self.seed);
        generator.generate(self.size, &mut rng)
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
        GenMeta {
            schema_version: GEN_META_SCHEMA_VERSION,
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            dtype: col.dtype(),
            count: col.len(),
            seed: spec.seed,
            shape: spec.shape.clone(),
            stats: col.stats(),
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

    #[test]
    fn uniform_is_reproducible_and_in_range() {
        let s = spec(
            Shape::Uniform {
                cardinality: 1000,
                dtype: DType::I64,
            },
            500,
            42,
        );
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
        let s = spec(
            Shape::Zipf {
                cardinality: 100,
                s: 1.1,
                dtype: DType::I64,
            },
            1000,
            7,
        );
        match s.generate().unwrap() {
            Column::I64(v) => assert!(v.iter().all(|x| (1..=100).contains(x))),
            _ => panic!("expected i64 column"),
        }
    }

    #[test]
    fn dtype_selects_physical_width() {
        let base = |dtype| {
            spec(
                Shape::Uniform {
                    cardinality: 256,
                    dtype,
                },
                64,
                1,
            )
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
            spec(
                Shape::Uniform {
                    cardinality: card,
                    dtype,
                },
                10_000,
                42,
            )
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
            spec(
                Shape::Uniform {
                    cardinality: 500,
                    dtype,
                },
                1_000,
                7,
            )
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
    fn zero_cardinality_is_rejected() {
        let err = Shape::Uniform {
            cardinality: 0,
            dtype: DType::I64,
        }
        .build();
        assert!(err.is_err());
    }

    #[test]
    fn meta_round_trips_through_json() {
        let s = spec(
            Shape::Zipf {
                cardinality: 50,
                s: 1.2,
                dtype: DType::U64,
            },
            100,
            9,
        );
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
        let json = r#"{"shape":"zipf","cardinality":1000,"s":1.1,"size":10,"seed":3}"#;
        let spec: GenSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.size, 10);
        assert_eq!(spec.seed, 3);
        assert!(matches!(spec.shape, Shape::Zipf { cardinality: 1000, .. }));
    }

    #[test]
    fn timestamp_is_monotonic_and_starts_at_start() {
        use gap::GapDist;
        let s = spec(
            Shape::MonotonicTimestamp {
                start: 1000,
                unit: TimeUnit::Nanos,
                gap: GapDist::Geometric { p: 0.01 },
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
        use gap::GapDist;
        let s = spec(
            Shape::MonotonicTimestamp {
                start: 0,
                unit: TimeUnit::Secs,
                gap: GapDist::Constant { step: 0 },
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
        use gap::GapDist;
        let s = spec(
            Shape::MonotonicTimestamp {
                start: i64::MAX - 5,
                unit: TimeUnit::Nanos,
                gap: GapDist::Constant { step: 100 },
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
        use weights::WeightSpec;
        let categories: Vec<i64> = (0..40).collect();
        let s = spec(
            Shape::SkewedCategorical {
                categories: categories.clone(),
                weights: WeightSpec::Zipf { s: 1.3 },
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
        use weights::WeightSpec;
        let err = Shape::SkewedCategorical {
            categories: vec![1, 2, 3],
            weights: WeightSpec::Explicit {
                weights: vec![1.0, 2.0],
            },
        }
        .build();
        assert!(err.is_err());
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
