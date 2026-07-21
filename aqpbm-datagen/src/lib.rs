//! Synthetic data-generation toolkit.
//!
//! A small, extensible library for producing benchmark workloads as
//! raw little-endian `.bin` files (consumed by `sketchlib bench
//! --input`) alongside a self-describing `.meta.json` sidecar.
//!
//! The design is intentionally decoupled from any particular benchmark:
//! generators write columns into a [`sink::Sink`], and a consumer reads
//! them back through the `.bin` + sidecar format, so no dispatch machinery
//! has to know a new distribution exists.
//!
//! This crate depends on nothing else in the workspace. The sketch
//! benchmark is its first consumer, not its owner -- which is why it is
//! named for the program it belongs to rather than for the sketches that
//! happen to read it today. See `docs/DESIGN.md` §4.3.
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
//! * New physical type → add a [`DType`] variant, one [`GenValue`] impl,
//!   and one arm in the caller's `match` on the requested dtype.
//!
//! ## Reproducibility
//!
//! Every generator is a pure function of `(spec, seed, n)`: it takes a
//! freshly seeded [`Xoshiro256PlusPlus`] and holds any running state
//! (e.g. a timestamp accumulator) as a call-local, so the same inputs
//! always yield byte-identical output.

pub mod dist;
pub mod error;
pub mod io;
pub mod shape;
pub mod sink;
pub mod stats;

use std::io::Write;
use std::path::Path;

use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

pub use crate::error::SketchError;

pub use dist::Distribution;
pub use shape::{Generator, Shape, TimeUnit};
pub use sink::{BinSink, MemorySink, Sink};
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
    /// Signed 64-bit. Every shape emits it, and it is the only type the
    /// `.bin` loader can read back.
    #[default]
    I64,
    /// Unsigned 64-bit. Emitted by `keys` and `monotonic`, but **nothing in
    /// this repo consumes it**: there is no `NumericItem` impl, so `bench`
    /// cannot ingest it, and the `.bin` loader rejects it. Generating one
    /// produces a file this workspace cannot read.
    U64,
    /// IEEE-754 double. `keys` only — `monotonic` rejects it — and readable
    /// only in-process via `bench --dtype f64`, not through `--input`.
    F64,
}

impl DType {
    /// Used by `WorkloadDesc`'s `skip_serializing_if` so an `i64` record keeps
    /// the exact bytes it had before the field existed.
    pub fn is_i64(&self) -> bool {
        matches!(self, DType::I64)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DType::I64 => "i64",
            DType::U64 => "u64",
            DType::F64 => "f64",
        }
    }
}

/// A value the generator can emit.
///
/// This is what replaced the `Column` enum. A column was "a `Vec` whose
/// element type is decided at run time", which forced every operation on
/// generated data — write, summarise, concatenate, unwrap — to be spelled
/// once per variant, 14 places in all. Adding a type meant editing all of
/// them, and a missed arm behind a `_` fallback compiled fine.
///
/// The run-time choice has not disappeared; a spec file really does say
/// `"dtype": "f64"` and something must act on that string. It moved to a
/// single `match` at the point the string is read, after which the whole
/// pipeline is one monomorphic `T`. Adding a type is now a `DType` variant,
/// one impl of this trait, and one arm in that match — and the match has no
/// `_` fallback, so a missing arm fails to compile.
pub trait GenValue: Copy + std::fmt::Debug + PartialEq + 'static {
    /// The tag recorded in the sidecar and the report.
    const DTYPE: DType;

    /// Render a raw `u64` draw as this type.
    ///
    /// Truncating, deliberately: the sampler's domain is bounded by
    /// `cardinality`, which `Shape::build` has already validated against the
    /// type (see `reject_inexact_f64`), so a draw always fits. Keeping this
    /// infallible is also what preserves the guarantee that a fixed
    /// `(shape, size, seed)` has the same *logical* values at every dtype —
    /// every type renders the same draw.
    fn from_draw(u: u64) -> Self;

    /// Narrow an accumulated `i128` (the monotonic series) to this type.
    ///
    /// Fallible, unlike [`Self::from_draw`], because an accumulator has no
    /// bound: a long series of large gaps really can leave the target type,
    /// and silently wrapping would produce a non-monotonic series.
    fn from_acc(a: i128) -> Result<Self, SketchError>;

    /// Value as `f64`, for the sidecar summary only.
    fn to_stats(self) -> f64;
}

/// A [`GenValue`] with a fixed byte width, and therefore writable to the
/// header-less `.bin` stream.
///
/// Separate from `GenValue` so the constraint sits on the one sink that has
/// it rather than on the generator. A variable-width value (a string) is a
/// perfectly good `GenValue` — it goes to memory, CSV, or anywhere else —
/// it just cannot be a `.bin`, and `BinSink::<That>` will not compile.
pub trait FixedWidth: GenValue {
    fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()>;
}

macro_rules! gen_value {
    ($ty:ty, $dtype:expr, $draw:expr) => {
        impl GenValue for $ty {
            const DTYPE: DType = $dtype;
            #[inline(always)]
            fn from_draw(u: u64) -> Self {
                #[allow(clippy::redundant_closure_call)]
                $draw(u)
            }
            fn from_acc(a: i128) -> Result<Self, SketchError> {
                <$ty>::try_from(a)
                    .map_err(|_| SketchError::BadParam("monotonic: value overflow".into()))
            }
            #[inline(always)]
            fn to_stats(self) -> f64 {
                self as f64
            }
        }

        impl FixedWidth for $ty {
            #[inline(always)]
            fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
                w.write_all(&self.to_le_bytes())
            }
        }
    };
}

gen_value!(i64, DType::I64, |u: u64| u as i64);
gen_value!(u64, DType::U64, |u: u64| u);

impl GenValue for f64 {
    const DTYPE: DType = DType::F64;
    #[inline(always)]
    fn from_draw(u: u64) -> Self {
        u as f64
    }
    /// `f64` has no `TryFrom<i128>`; bound it by the exact-integer range so a
    /// monotonic series cannot silently lose its last digits and stop being
    /// strictly increasing. (`Shape::build` rejects `f64` monotonic outright
    /// today, so this is the guard for if that ever changes, not dead weight
    /// covering a live path.)
    fn from_acc(a: i128) -> Result<Self, SketchError> {
        const LIMIT: i128 = 1 << 53;
        if a.abs() > LIMIT {
            return Err(SketchError::BadParam(
                "monotonic: value exceeds the f64 exact-integer range".into(),
            ));
        }
        Ok(a as f64)
    }
    #[inline(always)]
    fn to_stats(self) -> f64 {
        self
    }
}

impl FixedWidth for f64 {
    #[inline(always)]
    fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_all(&self.to_le_bytes())
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
    pub fn from_path(path: &Path) -> Result<Self, SketchError> {
        let text = std::fs::read_to_string(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("yaml") | Some("yml") => serde_yaml::from_str(&text)
                .map_err(|e| SketchError::BadParam(format!("spec yaml: {e}"))),
            _ => serde_json::from_str(&text)
                .map_err(|e| SketchError::BadParam(format!("spec json: {e}"))),
        }
    }

    /// Generate the whole column into memory.
    ///
    /// Convenience wrapper over [`Self::generate_into`] with a
    /// [`MemorySink`], for callers that want the values resident (the
    /// benchmark runner replays one slice per measured run) and know
    /// the dataset fits.
    pub fn generate<T: GenValue>(&self) -> Result<Vec<T>, SketchError> {
        let mut sink = MemorySink::<T>::new();
        self.generate_into(&mut sink, DEFAULT_CHUNK)?;
        Ok(sink.into_values())
    }

    /// Generate into `sink`, `chunk` values at a time, and return the
    /// provenance record for what was written.
    ///
    /// The chunking is what decouples dataset size from memory: a
    /// [`FileSink`] streams a dataset far larger than RAM, while a
    /// [`MemorySink`] reassembles the same bytes. `chunk` therefore
    /// affects only peak memory and never the output — see
    /// `chunk_size_does_not_change_output`.
    pub fn generate_into<T: GenValue, S: Sink<T>>(
        &self,
        sink: &mut S,
        chunk: usize,
    ) -> Result<GenMeta, SketchError> {
        use rand::SeedableRng;
        if chunk == 0 {
            return Err(SketchError::BadParam("chunk size must be > 0".into()));
        }
        // An empty workload benchmarks nothing, but every downstream stage
        // accepts it: the runner times an empty loop and reports `0.0
        // items/sec` with a confidence interval around it. `I64Workload::load`
        // already refuses a zero-item file; refuse the generated case here so
        // both sources agree.
        if self.size == 0 {
            return Err(SketchError::BadParam("size must be > 0".into()));
        }
        // The spec names a dtype and the caller names `T`. Disagreeing is an
        // error rather than a silent preference for either: obeying the spec
        // would ignore `--dtype`, and obeying `T` would edit the user's spec
        // file from the command line. Checked here, before any allocation,
        // rather than by unwrapping the finished data as it used to be.
        if T::DTYPE != self.shape.dtype() {
            return Err(SketchError::BadParam(format!(
                "spec generates {}, but {} was requested",
                self.shape.dtype().as_str(),
                T::DTYPE.as_str(),
            )));
        }
        let mut generator = self.shape.build::<T>()?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(self.seed);
        let mut stats = StatsAcc::new();

        let mut remaining = self.size;
        let mut buf: Vec<T> = Vec::with_capacity(chunk.min(self.size));
        while remaining > 0 {
            let n = remaining.min(chunk);
            buf.clear();
            generator.generate(n, &mut rng, &mut buf)?;
            stats.push_slice(&buf, |v| v.to_stats());
            sink.accept(&buf)?;
            remaining -= n;
        }
        sink.flush()?;

        Ok(GenMeta::from_parts(self, T::DTYPE, stats.finish()))
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
    /// Assemble the sidecar record for an already-materialised slice.
    pub fn new<T: GenValue>(spec: &GenSpec, values: &[T]) -> Self {
        let mut acc = StatsAcc::new();
        acc.push_slice(values, |v| v.to_stats());
        Self::from_parts(spec, T::DTYPE, acc.finish())
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
        let a = s.generate::<i64>().unwrap();
        let b = s.generate::<i64>().unwrap();
        assert_eq!(a, b, "same spec+seed must be byte-identical");
        assert_eq!(a.len(), 500);
        assert!(a.iter().all(|x| (0..1000).contains(x)));
    }

    #[test]
    fn zipf_ranks_in_expected_range() {
        let s = spec(
            keys(100, Distribution::Zipf { s: 1.1 }, DType::I64),
            1000,
            7,
        );
        let v = s.generate::<i64>().unwrap();
        assert!(v.iter().all(|x| (1..=100).contains(x)));
    }

    #[test]
    fn dtype_selects_physical_width() {
        // 64 values x 8 bytes each, every dtype. The dtype is now the type
        // parameter rather than a tag to match on, so asking for the wrong
        // one is a compile error and there is nothing left to assert about
        // which variant came back.
        fn written<T: GenValue + FixedWidth>(dtype: DType) -> usize {
            let v: Vec<T> = spec(keys(256, Distribution::Uniform, dtype), 64, 1)
                .generate()
                .unwrap();
            let mut buf = Vec::new();
            for x in &v {
                x.write_le(&mut buf).unwrap();
            }
            buf.len()
        }
        assert_eq!(written::<i64>(DType::I64), 64 * 8);
        assert_eq!(written::<u64>(DType::U64), 64 * 8);
        assert_eq!(written::<f64>(DType::F64), 64 * 8);
    }

    #[test]
    fn a_spec_and_a_requested_type_that_disagree_are_an_error() {
        // The guard that `Column::into_i64` used to perform after the fact.
        // Doing it up front means no data is generated to be thrown away,
        // and the message names both sides.
        let s = spec(keys(256, Distribution::Uniform, DType::I64), 64, 1);
        let err = s.generate::<f64>().unwrap_err().to_string();
        assert!(err.contains("i64") && err.contains("f64"), "{err}");
        assert!(s.generate::<i64>().is_ok());
    }

    #[test]
    fn uniform_cardinality_means_distinct_count_for_every_dtype() {
        // `cardinality` must not silently change meaning with dtype:
        // a continuous f64 range would yield ~n distinct values instead
        // of `cardinality`, quietly invalidating the one parameter a
        // sketch benchmark cares most about.
        let card = 100u64;
        let f64s: Vec<f64> = spec(keys(card, Distribution::Uniform, DType::F64), 10_000, 42)
            .generate()
            .unwrap();
        let distinct = f64s
            .iter()
            .map(|x| x.to_bits())
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(
            distinct, card as usize,
            "f64 must draw from `cardinality` keys"
        );
        assert!(
            f64s.iter()
                .all(|x| x.fract() == 0.0 && (0.0..card as f64).contains(x)),
            "f64 values must be whole numbers in [0, cardinality)"
        );
    }

    #[test]
    fn dtype_changes_encoding_not_logical_values() {
        // Holding shape+size+seed fixed, every dtype must produce the
        // same logical sequence — that is what makes dtype a controlled
        // variable when comparing benchmark runs.
        fn draw<T: GenValue>(dtype: DType) -> Vec<T> {
            spec(keys(500, Distribution::Uniform, dtype), 1_000, 7)
                .generate()
                .unwrap()
        }
        let i: Vec<i64> = draw(DType::I64);
        let u: Vec<u64> = draw(DType::U64);
        let f: Vec<f64> = draw(DType::F64);
        assert!(i.iter().zip(&u).all(|(a, b)| *a as u64 == *b));
        assert!(i.iter().zip(&f).all(|(a, b)| *a as f64 == *b));
    }

    #[test]
    fn f64_cardinality_past_2p53_is_rejected() {
        // Beyond 2^53 the `as f64` cast rounds, so the key space would
        // silently differ from the i64 run it is meant to mirror.
        let f64_build =
            |cardinality| keys(cardinality, Distribution::Uniform, DType::F64).build::<f64>();
        assert!(f64_build((1u64 << 53) + 1).is_err());
        assert!(f64_build(1u64 << 53).is_ok(), "the limit itself is exact");
        assert!(
            keys(u64::MAX, Distribution::Uniform, DType::I64)
                .build::<i64>()
                .is_ok(),
            "i64 is unaffected"
        );
        assert!(
            keys((1u64 << 53) + 1, Distribution::Zipf { s: 1.1 }, DType::F64)
                .build::<f64>()
                .is_err(),
            "zipf shares the limit"
        );
    }

    #[test]
    fn zero_cardinality_is_rejected() {
        assert!(keys(0, Distribution::Uniform, DType::I64)
            .build::<i64>()
            .is_err());
    }

    #[test]
    fn keys_reject_non_range_distribution() {
        // Geometric/poisson/explicit have no meaning as a key sampler.
        assert!(keys(100, Distribution::Poisson { lambda: 3.0 }, DType::I64)
            .build::<i64>()
            .is_err());
    }

    #[test]
    fn meta_round_trips_through_json() {
        let s = spec(keys(50, Distribution::Zipf { s: 1.2 }, DType::U64), 100, 9);
        let col = s.generate::<u64>().unwrap();
        let meta = GenMeta::new(&s, &col);
        let json = serde_json::to_string(&meta).unwrap();
        let back: GenMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shape, s.shape);
        assert_eq!(back.dtype, DType::U64);
        assert_eq!(back.count, 100);
    }

    #[test]
    fn genspec_deserializes_flattened_shape() {
        let json = r#"{"shape":"keys","cardinality":1000,"dist":{"kind":"zipf","s":1.1},"size":10,"seed":3}"#;
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
        let v = s.generate::<i64>().unwrap();
        assert_eq!(v[0], 1000, "first value must equal start");
        assert!(
            v.windows(2).all(|w| w[1] > w[0]),
            "min_gap=1 must be strictly increasing"
        );
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
        let v = s.generate::<u64>().unwrap();
        assert!(v.iter().all(|&x| x == 0), "constant-0 gap stays flat");
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
        assert!(
            s.generate::<i64>().is_err(),
            "accumulation past i64::MAX must error"
        );
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
        let v = s.generate::<i64>().unwrap();
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

    #[test]
    fn categorical_explicit_length_mismatch_errors() {
        let err = Shape::Categorical {
            categories: vec![1, 2, 3],
            dist: Distribution::Explicit {
                weights: vec![1.0, 2.0],
            },
        }
        .build::<i64>();
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
            let one_shot: Vec<i64> = {
                let mut sink = MemorySink::new();
                s.generate_into(&mut sink, 1 << 20).unwrap();
                sink.into_values()
            };
            let one_shot_stats = GenMeta::new(&s, &one_shot).stats;
            for chunk in [1usize, 7, 512, 4999, 5000] {
                let mut sink = MemorySink::<i64>::new();
                let meta = s.generate_into(&mut sink, chunk).unwrap();
                assert_eq!(
                    sink.into_values(),
                    one_shot,
                    "chunk={chunk} changed the values for {shape:?}"
                );
                assert_eq!(meta.count, 5_000, "chunk={chunk} lost rows");
                assert_eq!(
                    meta.stats, one_shot_stats,
                    "chunk={chunk} skewed the summary"
                );
            }
        }
    }

    #[test]
    fn bin_write_matches_le_layout() {
        // i64 bytes equal a hand-rolled LE encoding.
        let mut buf = Vec::new();
        for v in [1i64, -2, 3] {
            v.write_le(&mut buf).unwrap();
        }
        let mut expected = Vec::new();
        for v in [1i64, -2, 3] {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(buf, expected);
        let _ = Xoshiro256PlusPlus::seed_from_u64(0); // keep import used across cfgs
    }
}
