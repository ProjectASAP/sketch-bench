//! Synthetic data-generation toolkit: benchmark workloads as raw little-endian
//! `.bin` files plus a self-describing `.meta.json` sidecar. Distribution (*how*
//! values spread) and structure (*what* they mean) are orthogonal axes, and the
//! destination is a third — so a new one of any never touches the others. Every
//! generator is a pure function of `(spec, seed, n)`. See `docs/DESIGN.md` §4.3.

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

/// Values produced per [`Sink::accept`] call, bounding a streaming sink's memory
/// while keeping per-chunk dispatch to noise. A transport detail only: output is
/// byte-identical at any chunk size.
pub const DEFAULT_CHUNK: usize = 1 << 16;

/// Schema version of the `.meta.json` sidecar. Version 2 carries the orthogonal
/// structure / `Distribution` axes; v1's flattened shapes do not deserialize.
pub const GEN_META_SCHEMA_VERSION: u32 = 2;

/// A value the generator can emit. No run-time type tag: a caller names `T` and
/// the pipeline is one monomorphic instantiation, so adding a type is one impl
/// and nothing else. Only `workload generate`'s `--dtype` turns a string into one.
pub trait GenValue: Clone + std::fmt::Debug + PartialEq + 'static {
    /// Per-type rendering configuration, built once from the spec. `()` for the
    /// numeric types, where rendering a draw is a cast; a string needs an
    /// alphabet and a length rule, read from the spec at run time.
    type Cfg: Clone;

    /// The tag recorded in the sidecar and the report. Written once, here,
    /// so it cannot drift from the type it names.
    const NAME: &'static str;

    /// Largest integer this type holds exactly, if it has such a bound.
    /// `Some(2^53)` for `f64`; `None` for the integer types and strings.
    const EXACT_INTEGER_LIMIT: Option<u64> = None;

    /// Whether `Shape::Monotonic` can render into this type.
    const SUPPORTS_MONOTONIC: bool = true;

    /// Build this type's configuration from the spec, validating it eagerly
    /// so a bad `string:` block fails before any values are drawn.
    fn cfg(spec: &GenSpec) -> Result<Self::Cfg, SketchError>;

    /// Render a raw `u64` draw as this type, truncating for the numerics:
    /// `Shape::build` has bounded the sampler's domain against the type, so a
    /// draw always fits and every dtype renders the same logical values.
    fn from_draw(u: u64, cfg: &Self::Cfg) -> Self;

    /// Narrow an accumulated `i128` (the monotonic series) or a category id to
    /// this type. Fallible, unlike [`Self::from_draw`]: these are not bounded by
    /// `cardinality`, and wrapping would produce a non-monotonic series.
    fn from_acc(a: i128, cfg: &Self::Cfg) -> Result<Self, SketchError>;

    /// Value as `f64` for the sidecar summary, or `None` when it does not apply
    /// — a string's *length* under a field named `min` would be a lie in a
    /// provenance record, so a string column carries only `count`.
    fn stat(&self) -> Option<f64>;
}

/// A [`GenValue`] with a fixed byte width, therefore writable to the
/// header-less `.bin` stream. Separate from `GenValue` so the constraint sits on
/// the one sink that has it: `BinSink::<String>` simply will not compile.
pub trait FixedWidth: GenValue {
    fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()>;
}

macro_rules! gen_value {
    ($ty:ty, $name:literal, $draw:expr) => {
        impl GenValue for $ty {
            type Cfg = ();
            const NAME: &'static str = $name;
            fn cfg(_spec: &GenSpec) -> Result<(), SketchError> {
                Ok(())
            }
            #[inline(always)]
            fn from_draw(u: u64, _cfg: &()) -> Self {
                #[allow(clippy::redundant_closure_call)]
                $draw(u)
            }
            fn from_acc(a: i128, _cfg: &()) -> Result<Self, SketchError> {
                <$ty>::try_from(a)
                    .map_err(|_| SketchError::BadParam("monotonic: value overflow".into()))
            }
            #[inline(always)]
            fn stat(&self) -> Option<f64> {
                Some(*self as f64)
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

gen_value!(i64, "i64", |u: u64| u as i64);
gen_value!(u64, "u64", |u: u64| u);

impl GenValue for f64 {
    type Cfg = ();
    const NAME: &'static str = "f64";
    const EXACT_INTEGER_LIMIT: Option<u64> = Some(1 << 53);
    const SUPPORTS_MONOTONIC: bool = false;
    fn cfg(_spec: &GenSpec) -> Result<(), SketchError> {
        Ok(())
    }
    #[inline(always)]
    fn from_draw(u: u64, _cfg: &()) -> Self {
        u as f64
    }
    /// `f64` has no `TryFrom<i128>`; bound it by the exact-integer range so a
    /// monotonic series cannot lose its last digits and stop increasing.
    /// `Shape::build` rejects `f64` monotonic today, so this guards a change.
    fn from_acc(a: i128, _cfg: &()) -> Result<Self, SketchError> {
        const LIMIT: i128 = 1 << 53;
        if a.abs() > LIMIT {
            return Err(SketchError::BadParam(
                "monotonic: value exceeds the f64 exact-integer range".into(),
            ));
        }
        Ok(a as f64)
    }
    #[inline(always)]
    fn stat(&self) -> Option<f64> {
        Some(*self)
    }
}

/// How a drawn rank becomes a string. Only meaningful at `dtype: string`, and
/// ignored rather than rejected elsewhere, so one spec can run at several dtypes
/// — the point of dtype being a controlled variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringOpts {
    /// Characters strings are built from. Order matters: it is the digit
    /// order of the positional encoding below.
    #[serde(default = "default_alphabet")]
    pub alphabet: String,
    /// Inclusive length bounds. Equal values give fixed-length keys.
    #[serde(default = "default_min_len")]
    pub min_len: usize,
    #[serde(default = "default_max_len")]
    pub max_len: usize,
}

fn default_alphabet() -> String {
    "abcdefghijklmnopqrstuvwxyz0123456789".into()
}
fn default_min_len() -> usize {
    8
}
fn default_max_len() -> usize {
    24
}

impl Default for StringOpts {
    fn default() -> Self {
        Self {
            alphabet: default_alphabet(),
            min_len: default_min_len(),
            max_len: default_max_len(),
        }
    }
}

/// Validated [`StringOpts`] plus the prefix width injectivity requires.
#[derive(Debug, Clone)]
pub struct StrCfg {
    alphabet: Vec<char>,
    min_len: usize,
    max_len: usize,
    /// Leading characters positionally encoding the rank — a base-`|alphabet|`
    /// encoding, so two ranks always differ within them. That is what keeps the
    /// rendering **injective**, and the `cardinality` claim honest.
    prefix: usize,
}

impl StrCfg {
    fn build(opts: &StringOpts, cardinality: u64) -> Result<Self, SketchError> {
        let alphabet: Vec<char> = opts.alphabet.chars().collect();
        if alphabet.len() < 2 {
            return Err(SketchError::BadParam(
                "string: alphabet needs at least 2 distinct characters".into(),
            ));
        }
        {
            let mut seen: Vec<char> = alphabet.clone();
            seen.sort_unstable();
            seen.dedup();
            if seen.len() != alphabet.len() {
                return Err(SketchError::BadParam(
                    "string: alphabet has repeated characters, which would collapse distinct keys"
                        .into(),
                ));
            }
        }
        if opts.min_len == 0 {
            return Err(SketchError::BadParam("string: min_len must be > 0".into()));
        }
        if opts.min_len > opts.max_len {
            return Err(SketchError::BadParam(format!(
                "string: min_len {} exceeds max_len {}",
                opts.min_len, opts.max_len
            )));
        }
        // Smallest prefix width that can address `cardinality` distinct
        // ranks in this alphabet.
        let base = alphabet.len() as u128;
        let mut prefix = 1usize;
        let mut capacity = base;
        while capacity < cardinality as u128 {
            capacity *= base;
            prefix += 1;
        }
        if prefix > opts.max_len {
            return Err(SketchError::BadParam(format!(
                "string: cardinality {cardinality} needs at least {prefix} characters from a \
                 {}-character alphabet, but max_len is {}",
                alphabet.len(),
                opts.max_len
            )));
        }
        Ok(Self {
            alphabet,
            min_len: opts.min_len,
            max_len: opts.max_len,
            prefix,
        })
    }

    /// Deterministic per-rank scramble. Not an RNG: the length and the filler
    /// of a key must depend on the key alone, so the same rank renders to the
    /// same string every time it is drawn.
    fn mix(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }

    fn render(&self, rank: u64) -> String {
        let base = self.alphabet.len() as u64;
        let h = Self::mix(rank);
        // Length varies with the rank, never below the prefix the encoding
        // needs, so a short `min_len` narrows the spread instead of breaking
        // injectivity.
        let span = self.max_len - self.min_len + 1;
        let len = (self.min_len + (h % span as u64) as usize).max(self.prefix);

        let mut out = String::with_capacity(len);
        let mut r = rank;
        for _ in 0..self.prefix {
            out.push(self.alphabet[(r % base) as usize]);
            r /= base;
        }
        // Filler is derived from the rank too, so it carries no information
        // the prefix has not already fixed and cannot make two ranks collide.
        let mut f = h;
        for _ in self.prefix..len {
            f = Self::mix(f);
            out.push(self.alphabet[(f % base) as usize]);
        }
        out
    }
}

impl GenValue for String {
    type Cfg = StrCfg;
    const NAME: &'static str = "string";

    fn cfg(spec: &GenSpec) -> Result<StrCfg, SketchError> {
        let cardinality = spec.shape.domain_size().ok_or_else(|| {
            SketchError::BadParam(
                "string: needs a bounded key domain; `monotonic` has none, so its values \
                 cannot be rendered injectively"
                    .into(),
            )
        })?;
        StrCfg::build(
            spec.string.as_ref().unwrap_or(&StringOpts::default()),
            cardinality,
        )
    }

    #[inline]
    fn from_draw(u: u64, cfg: &StrCfg) -> Self {
        cfg.render(u)
    }

    /// Category ids are authored values, not ranks in `[0, cardinality)`, so
    /// rendering them through a prefix sized for the domain could collide.
    /// Refused rather than approximated.
    fn from_acc(_a: i128, _cfg: &StrCfg) -> Result<Self, SketchError> {
        Err(SketchError::BadParam(
            "string: only the `keys` shape generates strings".into(),
        ))
    }

    fn stat(&self) -> Option<f64> {
        None
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
    /// Rendering options for `dtype: string`. Absent means the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string: Option<StringOpts>,
}

impl GenSpec {
    /// Load a spec from a `.yaml`/`.yml` (serde_yaml) or otherwise JSON file,
    /// with the shape's tagged fields flattened alongside `size`/`seed` — e.g.
    /// `shape: keys`, `cardinality: 100000`, `dist: {kind: zipf, s: 1.1}`.
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

    /// Generate the whole column into memory — [`Self::generate_into`] over a
    /// [`MemorySink`], for callers that want the values resident (the runner
    /// replays one slice per measured run) and know the dataset fits.
    pub fn generate<T: GenValue>(&self) -> Result<Vec<T>, SketchError> {
        let mut sink = MemorySink::<T>::new();
        self.generate_into(&mut sink, DEFAULT_CHUNK)?;
        Ok(sink.into_values())
    }

    /// Generate into `sink`, `chunk` values at a time, returning the provenance
    /// record. Chunking decouples dataset size from memory, so `chunk` affects
    /// peak memory and never the output — `chunk_size_does_not_change_output`.
    pub fn generate_into<T: GenValue, S: Sink<T>>(
        &self,
        sink: &mut S,
        chunk: usize,
    ) -> Result<GenMeta, SketchError> {
        use rand::SeedableRng;
        if chunk == 0 {
            return Err(SketchError::BadParam("chunk size must be > 0".into()));
        }
        // An empty workload benchmarks nothing, yet every downstream stage
        // accepts it and reports `0.0 items/sec`. `I64Workload::load` refuses a
        // zero-item file; refuse the generated case so both sources agree.
        if self.size == 0 {
            return Err(SketchError::BadParam("size must be > 0".into()));
        }
        let cfg = T::cfg(self)?;
        let mut generator = self.shape.build::<T>(cfg)?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(self.seed);
        let mut stats = StatsAcc::new();

        let mut remaining = self.size;
        let mut buf: Vec<T> = Vec::with_capacity(chunk.min(self.size));
        while remaining > 0 {
            let n = remaining.min(chunk);
            buf.clear();
            generator.generate(n, &mut rng, &mut buf)?;
            stats.push_slice(&buf, |v| v.stat());
            sink.accept(&buf)?;
            remaining -= n;
        }
        sink.flush()?;

        Ok(GenMeta::from_parts(self, T::NAME, stats.finish()))
    }
}

/// Provenance written to the `foo.bin.meta.json` sidecar. Carries the
/// full [`Shape`] (round-trips exactly) so `describe` can recover how a
/// file was made even though the `.bin` itself is header-less.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenMeta {
    pub schema_version: u32,
    pub generator_version: String,
    pub dtype: String,
    pub count: usize,
    pub seed: u64,
    pub shape: Shape,
    pub stats: BasicStats,
}

impl GenMeta {
    /// Assemble the sidecar record for an already-materialised slice.
    pub fn new<T: GenValue>(spec: &GenSpec, values: &[T]) -> Self {
        let mut acc = StatsAcc::new();
        acc.push_slice(values, |v| v.stat());
        Self::from_parts(spec, T::NAME, acc.finish())
    }

    /// Assemble the sidecar record from a streamed generation, where the
    /// values were never all resident to summarise in one pass.
    pub fn from_parts(spec: &GenSpec, dtype: &str, stats: BasicStats) -> Self {
        GenMeta {
            schema_version: GEN_META_SCHEMA_VERSION,
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            dtype: dtype.to_string(),
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
        GenSpec {
            shape,
            size,
            seed,
            string: None,
        }
    }

    fn keys(cardinality: u64, dist: Distribution) -> Shape {
        Shape::Keys { cardinality, dist }
    }

    #[test]
    fn uniform_is_reproducible_and_in_range() {
        let s = spec(keys(1000, Distribution::Uniform), 500, 42);
        let a = s.generate::<i64>().unwrap();
        let b = s.generate::<i64>().unwrap();
        assert_eq!(a, b, "same spec+seed must be byte-identical");
        assert_eq!(a.len(), 500);
        assert!(a.iter().all(|x| (0..1000).contains(x)));
    }

    #[test]
    fn zipf_ranks_in_expected_range() {
        let s = spec(keys(100, Distribution::Zipf { s: 1.1 }), 1000, 7);
        let v = s.generate::<i64>().unwrap();
        assert!(v.iter().all(|x| (1..=100).contains(x)));
    }

    #[test]
    fn the_type_parameter_selects_physical_width() {
        // 64 values x 8 bytes each, for every type. The type parameter is now
        // the only thing that selects an encoding — there is no tag to
        // disagree with it.
        fn written<T: GenValue + FixedWidth>() -> usize {
            let v: Vec<T> = spec(keys(256, Distribution::Uniform), 64, 1)
                .generate()
                .unwrap();
            let mut buf = Vec::new();
            for x in &v {
                x.write_le(&mut buf).unwrap();
            }
            buf.len()
        }
        assert_eq!(written::<i64>(), 64 * 8);
        assert_eq!(written::<u64>(), 64 * 8);
        assert_eq!(written::<f64>(), 64 * 8);
    }

    #[test]
    fn uniform_cardinality_means_distinct_count_for_every_item_type() {
        // `cardinality` must not change meaning with the item type: a continuous
        // f64 range would yield ~n distinct values instead of `cardinality`,
        // invalidating the parameter the benchmark is built around.
        let card = 100u64;
        let f64s: Vec<f64> = spec(keys(card, Distribution::Uniform), 10_000, 42)
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
    fn the_item_type_changes_encoding_not_logical_values() {
        // Holding shape+size+seed fixed, every type must produce the same
        // logical sequence — that is what makes the item type a controlled
        // variable when comparing benchmark runs.
        fn draw<T: GenValue>() -> Vec<T> {
            spec(keys(500, Distribution::Uniform), 1_000, 7)
                .generate()
                .unwrap()
        }
        let i: Vec<i64> = draw();
        let u: Vec<u64> = draw();
        let f: Vec<f64> = draw();
        assert!(i.iter().zip(&u).all(|(a, b)| *a as u64 == *b));
        assert!(i.iter().zip(&f).all(|(a, b)| *a as f64 == *b));
    }

    #[test]
    fn f64_cardinality_past_2p53_is_rejected() {
        // Beyond 2^53 the `as f64` cast rounds, so the key space would
        // silently differ from the i64 run it is meant to mirror.
        let f64_build = |cardinality| keys(cardinality, Distribution::Uniform).build::<f64>(());
        assert!(f64_build((1u64 << 53) + 1).is_err());
        assert!(f64_build(1u64 << 53).is_ok(), "the limit itself is exact");
        assert!(
            keys(u64::MAX, Distribution::Uniform)
                .build::<i64>(())
                .is_ok(),
            "i64 is unaffected"
        );
        assert!(
            keys((1u64 << 53) + 1, Distribution::Zipf { s: 1.1 })
                .build::<f64>(())
                .is_err(),
            "zipf shares the limit"
        );
    }

    #[test]
    fn zero_cardinality_is_rejected() {
        assert!(keys(0, Distribution::Uniform).build::<i64>(()).is_err());
    }

    #[test]
    fn keys_reject_non_range_distribution() {
        // Geometric/poisson/explicit have no meaning as a key sampler.
        assert!(keys(100, Distribution::Poisson { lambda: 3.0 })
            .build::<i64>(())
            .is_err());
    }

    #[test]
    fn meta_round_trips_through_json() {
        let s = spec(keys(50, Distribution::Zipf { s: 1.2 }), 100, 9);
        let col = s.generate::<u64>().unwrap();
        let meta = GenMeta::new(&s, &col);
        let json = serde_json::to_string(&meta).unwrap();
        let back: GenMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shape, s.shape);
        assert_eq!(back.dtype, "u64");
        assert_eq!(back.count, 100);
    }

    /// A spec file written before the item type became a type parameter
    /// still carries `dtype:`. It must keep loading — the field is now
    /// ignored, not rejected, so existing files do not have to be edited.
    #[test]
    fn a_spec_file_with_a_stale_dtype_still_loads() {
        let yaml = "shape: monotonic\nstart: 1700000000000\nunit: millis\ngap:\n  kind: exponential\n  lambda: 0.5\nmin_gap: 1\ndtype: i64\nsize: 1000\nseed: 42\n";
        let spec: GenSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.size, 1000);
        assert!(matches!(spec.shape, Shape::Monotonic { min_gap: 1, .. }));

        let json = r#"{"shape":"keys","cardinality":100,"dist":{"kind":"uniform"},"dtype":"f64","size":10,"seed":3}"#;
        let spec: GenSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.size, 10);
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
        .build::<i64>(());
        assert!(err.is_err());
    }

    /// Chunking is a property of the *transport*, never of the data.
    /// If it were not, the file sink and the in-memory benchmark path
    /// would silently disagree about what "the same workload" means.
    #[test]
    fn chunk_size_does_not_change_output() {
        let shapes = [
            keys(1000, Distribution::Zipf { s: 1.1 }),
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

#[cfg(test)]
mod string_tests {
    use super::*;

    fn spec(cardinality: u64, size: usize, opts: Option<StringOpts>) -> GenSpec {
        GenSpec {
            shape: Shape::Keys {
                cardinality,
                dist: Distribution::Uniform,
            },
            size,
            seed: 42,
            string: opts,
        }
    }

    /// The property everything rests on: if two ranks rendered to the same
    /// string, the run would have fewer distinct keys than it claims and every
    /// per-key error figure would use the wrong denominator.
    #[test]
    fn distinct_ranks_never_collide() {
        let card = 5_000u64;
        let s = spec(card, 200_000, None);
        let v: Vec<String> = s.generate().unwrap();
        let distinct: std::collections::HashSet<&String> = v.iter().collect();
        assert_eq!(
            distinct.len(),
            card as usize,
            "rendering must be injective over the key domain"
        );
    }

    /// A key must render the same way every time it is drawn, or the "same
    /// key" is a different key on each occurrence and every sketch sees a
    /// cardinality equal to the stream length.
    #[test]
    fn a_rank_always_renders_to_the_same_string() {
        let s = spec(64, 4_000, None);
        let v: Vec<String> = s.generate().unwrap();
        let mut seen: std::collections::HashMap<usize, &String> = std::collections::HashMap::new();
        // The same spec at i64 gives the ranks behind those strings.
        let ranks: Vec<i64> = spec(64, 4_000, None).generate().unwrap();
        for (r, sv) in ranks.iter().zip(&v) {
            if let Some(prev) = seen.insert(*r as usize, sv) {
                assert_eq!(prev, sv, "rank {r} rendered two different ways");
            }
        }
        assert!(seen.len() > 1, "test needs repeated keys to be meaningful");
    }

    /// The whole point of a string workload: lengths vary, so hash and
    /// comparison cost vary with them — the axis that decimal-formatted
    /// integers, at 1-7 characters over 10 symbols, could not move.
    #[test]
    fn lengths_spread_across_the_configured_range() {
        let opts = StringOpts {
            alphabet: "abcdefghijklmnopqrstuvwxyz".into(),
            min_len: 6,
            max_len: 20,
        };
        let v: Vec<String> = spec(10_000, 50_000, Some(opts)).generate().unwrap();
        let lens: std::collections::HashSet<usize> = v.iter().map(|s| s.len()).collect();
        assert!(
            lens.iter().all(|&l| (6..=20).contains(&l)),
            "lengths out of range: {lens:?}"
        );
        assert!(
            lens.len() >= 10,
            "expected a spread of lengths, saw {}",
            lens.len()
        );
    }

    #[test]
    fn the_alphabet_is_respected() {
        let opts = StringOpts {
            alphabet: "01".into(),
            min_len: 16,
            max_len: 16,
        };
        let v: Vec<String> = spec(1_000, 5_000, Some(opts)).generate().unwrap();
        assert!(v
            .iter()
            .all(|s| s.len() == 16 && s.chars().all(|c| c == '0' || c == '1')));
    }

    #[test]
    fn an_alphabet_too_small_for_the_cardinality_is_rejected() {
        // 2^8 = 256 < 1000, so 8 characters cannot address the domain.
        let opts = StringOpts {
            alphabet: "01".into(),
            min_len: 4,
            max_len: 8,
        };
        let err = spec(1_000, 10, Some(opts))
            .generate::<String>()
            .unwrap_err()
            .to_string();
        assert!(err.contains("max_len"), "{err}");
    }

    #[test]
    fn malformed_options_are_rejected_by_name() {
        for (opts, needle) in [
            (
                StringOpts {
                    alphabet: "aab".into(),
                    min_len: 4,
                    max_len: 8,
                },
                "repeated",
            ),
            (
                StringOpts {
                    alphabet: "a".into(),
                    min_len: 4,
                    max_len: 8,
                },
                "at least 2",
            ),
            (
                StringOpts {
                    alphabet: "abc".into(),
                    min_len: 9,
                    max_len: 8,
                },
                "exceeds",
            ),
        ] {
            let err = spec(100, 10, Some(opts))
                .generate::<String>()
                .unwrap_err()
                .to_string();
            assert!(err.contains(needle), "expected {needle:?} in {err:?}");
        }
    }

    /// Monotonic has no bounded domain, so there is no prefix width that
    /// keeps rendering injective. Refused up front rather than silently
    /// producing colliding keys.
    #[test]
    fn a_monotonic_string_series_is_refused() {
        let s = GenSpec {
            shape: Shape::Monotonic {
                start: 0,
                unit: TimeUnit::Nanos,
                gap: Distribution::Constant { value: 1 },
                min_gap: 1,
            },
            size: 10,
            seed: 1,
            string: None,
        };
        assert!(s.generate::<String>().is_err());
    }

    /// A string column has no numeric min/max. Reporting its length under
    /// those names would be a lie in a provenance record, so they are absent
    /// and only `count` is carried.
    #[test]
    fn the_sidecar_summary_omits_numbers_it_does_not_have() {
        let s = spec(100, 500, None);
        let v: Vec<String> = s.generate().unwrap();
        let meta = GenMeta::new(&s, &v);
        assert_eq!(meta.count, 500);
        assert_eq!(meta.dtype, "string");
        assert!(meta.stats.min.is_none() && meta.stats.max.is_none());
        assert_eq!(meta.stats.count, 500);
    }
}
