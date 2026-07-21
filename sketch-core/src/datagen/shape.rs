//! The structure axis — *what* the generated values mean, orthogonal
//! to the [`Distribution`] that says *how* they are spread.
//!
//! Each variant owns a [`Distribution`] and the domain it applies it
//! over; the distribution appears once (in `dist.rs`) rather than once
//! per structure. Adding a structure is a `*Gen` struct, a
//! [`Generator`] variant, and a variant + build arm here.

use rand::distributions::WeightedIndex;
use rand_distr::Distribution as _;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;
use crate::workload::WorkloadDesc;

use super::dist::{Distribution, GapSampler, KeySampler};
use super::{Column, DType};

/// A semantic label for the units of a timestamp column. Metadata only:
/// it does not rescale generated values (gaps are measured in these
/// units directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeUnit {
    #[default]
    Nanos,
    Millis,
    Secs,
}

fn default_min_gap() -> u64 {
    1
}

fn default_dist() -> Distribution {
    Distribution::Uniform
}

/// `f64` represents integers exactly only up to 2^53. Past that, a
/// `cardinality`-sized key space would silently collapse onto rounded
/// values — the same class of quiet wrongness the dtype guard in
/// [`crate::workload::FileI64`] exists to prevent — so reject it.
fn reject_inexact_f64(dtype: DType, cardinality: u64) -> Result<(), SketchCoreError> {
    const LIMIT: u64 = 1 << 53;
    if dtype == DType::F64 && cardinality > LIMIT {
        return Err(SketchCoreError::BadParam(format!(
            "dtype f64 cannot hold {cardinality} distinct integers exactly \
             (limit 2^53 = {LIMIT}); values would round silently"
        )));
    }
    Ok(())
}

/// A generation structure: a domain plus the [`Distribution`] drawn over
/// it. `#[serde(tag = "shape")]` makes this the canonical (de)serialized
/// form used by `GenSpec`/`GenMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// A large key space of `cardinality` distinct keys drawn per
    /// `dist` (which must be range-valued: uniform / zipf / constant).
    /// The exact base is per-dist — uniform covers `[0, cardinality)`,
    /// zipf covers ranks `[1, cardinality]` — but the count is always
    /// `cardinality`. That count is the same for every dtype, so `f64`
    /// emits whole numbers: the same logical values as `i64`/`u64`,
    /// differing only in physical encoding, so a benchmark can vary
    /// `dtype` alone.
    Keys {
        cardinality: u64,
        #[serde(default = "default_dist")]
        dist: Distribution,
        #[serde(default)]
        dtype: DType,
    },
    /// A finite domain of category ids, drawn per `dist` (uniform / zipf
    /// / explicit weights). Always emits `i64`.
    Categorical {
        categories: Vec<i64>,
        #[serde(default = "default_dist")]
        dist: Distribution,
    },
    /// A monotonically non-decreasing series (e.g. event timestamps):
    /// each value is the previous plus a non-negative gap drawn from
    /// `gap` (a count/interval distribution). `min_gap` floors each gap
    /// (1 → strictly increasing, 0 → duplicates allowed).
    Monotonic {
        #[serde(default)]
        start: i64,
        #[serde(default)]
        unit: TimeUnit,
        gap: Distribution,
        #[serde(default = "default_min_gap")]
        min_gap: u64,
        #[serde(default)]
        dtype: DType,
    },
}

impl Shape {
    /// The physical output type this shape emits.
    pub fn dtype(&self) -> DType {
        match self {
            Shape::Keys { dtype, .. } | Shape::Monotonic { dtype, .. } => *dtype,
            Shape::Categorical { .. } => DType::I64,
        }
    }

    /// The label used in reports and CLI output. This is a
    /// back-compat shim, **not** a clean structural tag: for `Keys` it
    /// returns the *distribution* name (`"uniform"`/`"zipf"`/…) so
    /// existing plot scripts that filter on `shape=="zipf"` keep
    /// working, while the other structures return a structural name.
    /// Do not rely on it to identify the structure axis; use the enum
    /// variant for that.
    pub fn report_label(&self) -> &'static str {
        match self {
            Shape::Keys { dist, .. } => dist.tag(),
            Shape::Categorical { .. } => "skewed_categorical",
            Shape::Monotonic { .. } => "monotonic_timestamp",
        }
    }

    /// Build the concrete generator, validating parameters eagerly so
    /// bad specs fail before any allocation.
    pub fn build(&self) -> Result<Generator, SketchCoreError> {
        match self {
            Shape::Keys {
                cardinality,
                dist,
                dtype,
            } => {
                if *cardinality == 0 {
                    return Err(SketchCoreError::BadParam(
                        "keys: cardinality must be > 0".into(),
                    ));
                }
                reject_inexact_f64(*dtype, *cardinality)?;
                Ok(Generator::Keys(KeysGen {
                    sampler: dist.key_sampler(*cardinality)?,
                    dtype: *dtype,
                }))
            }
            Shape::Categorical { categories, dist } => {
                if categories.is_empty() {
                    return Err(SketchCoreError::BadParam(
                        "categorical: categories must be non-empty".into(),
                    ));
                }
                let weights = dist.weights(categories.len())?;
                let index = WeightedIndex::new(&weights)
                    .map_err(|e| SketchCoreError::BadParam(format!("weighted index: {e}")))?;
                Ok(Generator::Categorical(CategoricalGen {
                    categories: categories.clone(),
                    index,
                }))
            }
            Shape::Monotonic {
                start,
                gap,
                min_gap,
                dtype,
                unit: _,
            } => {
                if *dtype == DType::F64 {
                    return Err(SketchCoreError::BadParam(
                        "monotonic: dtype f64 unsupported; use i64 or u64".into(),
                    ));
                }
                Ok(Generator::Monotonic(MonotonicGen {
                    start: *start,
                    gap: gap.gap_sampler()?,
                    min_gap: *min_gap,
                    dtype: *dtype,
                }))
            }
        }
    }

    /// Lossy projection into the report-facing [`WorkloadDesc`] so JSONL
    /// records stay well-formed regardless of shape.
    pub fn to_workload_desc(&self, size: usize, seed: u64) -> WorkloadDesc {
        let (cardinality, zipf_s) = match self {
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
            shape: self.report_label().to_string(),
            size,
            cardinality,
            zipf_s,
            source_path: None,
            seed: Some(seed),
        }
    }
}

/// A prepared generator: the closed set of structures, one per `Shape`
/// variant. Built by [`Shape::build`], driven by [`GenSpec::generate`].
/// An enum rather than a trait object — the set is closed and
/// crate-private, matching how every other datagen concern
/// ([`Distribution`], [`Column`], [`Shape`]) is modelled.
pub enum Generator {
    Keys(KeysGen),
    Categorical(CategoricalGen),
    Monotonic(MonotonicGen),
}

impl Generator {
    pub fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        match self {
            Generator::Keys(g) => g.generate(n, rng),
            Generator::Categorical(g) => g.generate(n, rng),
            Generator::Monotonic(g) => g.generate(n, rng),
        }
    }
}

/// Keys drawn over the sampler's domain, emitted in the requested
/// physical dtype. Every dtype casts the *same* `u64` draw, so a fixed
/// `(shape, size, seed)` is dtype-invariant in its logical values.
pub struct KeysGen {
    sampler: KeySampler,
    dtype: DType,
}

impl KeysGen {
    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        Ok(match self.dtype {
            DType::I64 => Column::I64((0..n).map(|_| self.sampler.sample(rng) as i64).collect()),
            DType::U64 => Column::U64((0..n).map(|_| self.sampler.sample(rng)).collect()),
            DType::F64 => Column::F64((0..n).map(|_| self.sampler.sample(rng) as f64).collect()),
        })
    }
}

/// Draws category ids from a fixed domain with a configurable skew.
pub struct CategoricalGen {
    categories: Vec<i64>,
    index: WeightedIndex<f64>,
}

impl CategoricalGen {
    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        Ok(Column::I64(
            (0..n)
                .map(|_| self.categories[self.index.sample(rng)])
                .collect(),
        ))
    }
}

/// Monotonically non-decreasing values built by accumulating gaps. The
/// accumulator is an `i128` call-local so a fresh-seeded call is
/// bit-reproducible; overflow past the target type is a hard error.
pub struct MonotonicGen {
    start: i64,
    gap: GapSampler,
    min_gap: u64,
    dtype: DType,
}

impl MonotonicGen {
    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        // Accumulate once in the widest integer, then narrow to the
        // target dtype in a single trailing match — no per-dtype loop.
        let mut acc: i128 = self.start as i128;
        let raw: Vec<i128> = (0..n)
            .map(|i| {
                if i > 0 {
                    acc += self.gap.sample_ticks(rng).max(self.min_gap) as i128;
                }
                acc
            })
            .collect();
        let overflow = || SketchCoreError::BadParam("monotonic: value overflow".into());
        Ok(match self.dtype {
            DType::U64 => Column::U64(
                raw.iter()
                    .map(|&a| u64::try_from(a).map_err(|_| overflow()))
                    .collect::<Result<_, _>>()?,
            ),
            // f64 is rejected in `build`; treat anything non-u64 as i64.
            _ => Column::I64(
                raw.iter()
                    .map(|&a| i64::try_from(a).map_err(|_| overflow()))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}
