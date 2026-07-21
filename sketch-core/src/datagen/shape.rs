//! The structure axis — *what* the generated values mean, orthogonal
//! to the [`Distribution`] that says *how* they are spread.
//!
//! Each variant owns a [`Distribution`] and the domain it applies it
//! over; the distribution appears once (in `dist.rs`) rather than once
//! per structure. Adding a structure is a struct implementing
//! [`ColumnGenerator`], a variant here, and an arm in [`Shape::build`].

use rand::distributions::WeightedIndex;
use rand_distr::Distribution as _;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;
use crate::workload::WorkloadDesc;

use super::dist::{Distribution, GapSampler, KeySampler};
use super::{Column, ColumnGenerator, DType};

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
    /// A large key space: `cardinality` distinct keys `[0, cardinality)`,
    /// drawn per `dist`. `dist` must be range-valued (uniform / zipf /
    /// constant). `cardinality` means the distinct-key count for every
    /// dtype, so `f64` emits whole numbers — the same logical values as
    /// `i64`/`u64`, differing only in physical encoding, so a benchmark
    /// can vary `dtype` alone.
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

    /// A short shape tag for reports and CLI output. For `Keys` this is
    /// the distribution name, preserving the historical `"uniform"` /
    /// `"zipf"` report strings.
    pub fn tag(&self) -> &'static str {
        match self {
            Shape::Keys { dist, .. } => dist.tag(),
            Shape::Categorical { .. } => "skewed_categorical",
            Shape::Monotonic { .. } => "monotonic_timestamp",
        }
    }

    /// Build the concrete generator, validating parameters eagerly so
    /// bad specs fail before any allocation.
    pub fn build(&self) -> Result<Box<dyn ColumnGenerator>, SketchCoreError> {
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
                Ok(Box::new(KeysGen {
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
                Ok(Box::new(CategoricalGen {
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
                Ok(Box::new(MonotonicGen {
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
            shape: self.tag().to_string(),
            size,
            cardinality,
            zipf_s,
            source_path: None,
            seed: Some(seed),
        }
    }
}

/// Keys drawn over `[0, cardinality)` per the prepared sampler, emitted
/// in the requested physical dtype. Every dtype casts the *same* `u64`
/// draw, so a fixed `(shape, size, seed)` is dtype-invariant in its
/// logical values.
struct KeysGen {
    sampler: KeySampler,
    dtype: DType,
}

impl ColumnGenerator for KeysGen {
    fn dtype(&self) -> DType {
        self.dtype
    }

    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        Ok(match self.dtype {
            DType::I64 => Column::I64((0..n).map(|_| self.sampler.sample(rng) as i64).collect()),
            DType::U64 => Column::U64((0..n).map(|_| self.sampler.sample(rng)).collect()),
            DType::F64 => Column::F64((0..n).map(|_| self.sampler.sample(rng) as f64).collect()),
        })
    }
}

/// Draws category ids from a fixed domain with a configurable skew.
struct CategoricalGen {
    categories: Vec<i64>,
    index: WeightedIndex<f64>,
}

impl ColumnGenerator for CategoricalGen {
    fn dtype(&self) -> DType {
        DType::I64
    }

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
struct MonotonicGen {
    start: i64,
    gap: GapSampler,
    min_gap: u64,
    dtype: DType,
}

impl ColumnGenerator for MonotonicGen {
    fn dtype(&self) -> DType {
        self.dtype
    }

    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        let overflow = || SketchCoreError::BadParam("monotonic: value overflow".into());
        let mut acc: i128 = self.start as i128;
        let mut i64s = Vec::new();
        let mut u64s = Vec::new();
        let want_u64 = self.dtype == DType::U64;
        if want_u64 {
            u64s.reserve(n);
        } else {
            i64s.reserve(n);
        }
        for i in 0..n {
            if i > 0 {
                acc += self.gap.sample_ticks(rng).max(self.min_gap) as i128;
            }
            if want_u64 {
                u64s.push(u64::try_from(acc).map_err(|_| overflow())?);
            } else {
                i64s.push(i64::try_from(acc).map_err(|_| overflow())?);
            }
        }
        Ok(if want_u64 {
            Column::U64(u64s)
        } else {
            Column::I64(i64s)
        })
    }
}
