//! The `Shape` registry — the serde-tagged front door and factory for
//! every distribution the generator can produce.
//!
//! Adding a distribution is a three-line change with no impact on the
//! benchmark: add a struct implementing [`ColumnGenerator`], add a
//! variant here, and add its arm to [`Shape::build`].

use rand::distributions::WeightedIndex;
use rand_distr::{Distribution, Uniform, Zipf};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;
use crate::workload::WorkloadDesc;

use super::gap::{GapDist, GapSampler};
use super::weights::WeightSpec;
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

/// A distribution plus its parameters. `#[serde(tag = "shape")]` makes
/// this the canonical (de)serialized form used by `GenSpec`/`GenMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// Uniform over the `cardinality` distinct keys `[0, cardinality)`.
    ///
    /// `cardinality` means the same thing for every `dtype`: the number
    /// of distinct values drawn from. `f64` therefore emits whole
    /// numbers (`0.0`, `1.0`, …), not a continuous range — the same
    /// logical values as `i64`/`u64`, differing only in physical
    /// encoding, so a benchmark can vary `dtype` alone. A genuinely
    /// continuous real-valued shape (for quantile sketches) belongs in
    /// its own variant rather than overloading this one.
    Uniform {
        cardinality: u64,
        #[serde(default)]
        dtype: DType,
    },
    /// Zipfian over ranks `[1, cardinality]` with skew exponent `s`.
    Zipf {
        cardinality: u64,
        s: f64,
        #[serde(default)]
        dtype: DType,
    },
    /// Monotonically non-decreasing values (e.g. event timestamps): each
    /// value is the previous one plus a positive integer gap drawn from
    /// `gap`. `min_gap` floors each gap (1 → strictly increasing, 0 →
    /// duplicates allowed).
    MonotonicTimestamp {
        #[serde(default)]
        start: i64,
        #[serde(default)]
        unit: TimeUnit,
        gap: GapDist,
        #[serde(default = "default_min_gap")]
        min_gap: u64,
        #[serde(default)]
        dtype: DType,
    },
    /// Draws from a fixed set of category ids with a configurable skew
    /// over which ids are common (e.g. a few dozen OS types).
    SkewedCategorical {
        categories: Vec<i64>,
        weights: WeightSpec,
    },
}

impl Shape {
    /// The physical output type this shape emits.
    pub fn dtype(&self) -> DType {
        match self {
            Shape::Uniform { dtype, .. }
            | Shape::Zipf { dtype, .. }
            | Shape::MonotonicTimestamp { dtype, .. } => *dtype,
            Shape::SkewedCategorical { .. } => DType::I64,
        }
    }

    /// A short shape tag for reports and CLI output.
    pub fn tag(&self) -> &'static str {
        match self {
            Shape::Uniform { .. } => "uniform",
            Shape::Zipf { .. } => "zipf",
            Shape::MonotonicTimestamp { .. } => "monotonic_timestamp",
            Shape::SkewedCategorical { .. } => "skewed_categorical",
        }
    }

    /// Build the concrete generator, validating parameters eagerly so
    /// bad specs fail before any allocation.
    pub fn build(&self) -> Result<Box<dyn ColumnGenerator>, SketchCoreError> {
        match self {
            Shape::Uniform { cardinality, dtype } => {
                if *cardinality == 0 {
                    return Err(SketchCoreError::BadParam(
                        "uniform: cardinality must be > 0".into(),
                    ));
                }
                reject_inexact_f64(*dtype, *cardinality)?;
                Ok(Box::new(UniformGen {
                    cardinality: *cardinality,
                    dtype: *dtype,
                }))
            }
            Shape::Zipf {
                cardinality,
                s,
                dtype,
            } => {
                if *cardinality == 0 {
                    return Err(SketchCoreError::BadParam(
                        "zipf: cardinality must be > 0".into(),
                    ));
                }
                reject_inexact_f64(*dtype, *cardinality)?;
                // Surface an invalid `s` now rather than mid-generation.
                Zipf::new(*cardinality, *s)
                    .map_err(|e| SketchCoreError::BadParam(format!("zipf: {e}")))?;
                Ok(Box::new(ZipfGen {
                    cardinality: *cardinality,
                    s: *s,
                    dtype: *dtype,
                }))
            }
            Shape::MonotonicTimestamp {
                start,
                gap,
                min_gap,
                dtype,
                unit: _,
            } => {
                if *dtype == DType::F64 {
                    return Err(SketchCoreError::BadParam(
                        "monotonic_timestamp: dtype f64 unsupported; use i64 or u64".into(),
                    ));
                }
                Ok(Box::new(TimestampGen {
                    start: *start,
                    gap: gap.build()?,
                    min_gap: *min_gap,
                    dtype: *dtype,
                }))
            }
            Shape::SkewedCategorical {
                categories,
                weights,
            } => {
                if categories.is_empty() {
                    return Err(SketchCoreError::BadParam(
                        "skewed_categorical: categories must be non-empty".into(),
                    ));
                }
                let resolved = weights.resolve(categories.len())?;
                let index = WeightedIndex::new(&resolved)
                    .map_err(|e| SketchCoreError::BadParam(format!("weighted index: {e}")))?;
                Ok(Box::new(SkewedCategoricalGen {
                    categories: categories.clone(),
                    index,
                }))
            }
        }
    }

    /// Lossy projection into the report-facing [`WorkloadDesc`] so JSONL
    /// records stay well-formed regardless of shape.
    pub fn to_workload_desc(&self, size: usize, seed: u64) -> WorkloadDesc {
        let (cardinality, zipf_s) = match self {
            Shape::Uniform { cardinality, .. } => (Some(*cardinality), None),
            Shape::Zipf { cardinality, s, .. } => (Some(*cardinality), Some(*s)),
            Shape::MonotonicTimestamp { .. } => (None, None),
            Shape::SkewedCategorical { categories, .. } => (Some(categories.len() as u64), None),
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

/// Uniform over `[0, cardinality)`. Mirrors `workload::UniformI64` and
/// generalizes it over the physical dtype.
struct UniformGen {
    cardinality: u64,
    dtype: DType,
}

impl ColumnGenerator for UniformGen {
    fn dtype(&self) -> DType {
        self.dtype
    }

    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        Ok(match self.dtype {
            DType::I64 => {
                let dist = Uniform::new(0u64, self.cardinality);
                Column::I64((0..n).map(|_| dist.sample(rng) as i64).collect())
            }
            DType::U64 => {
                let dist = Uniform::new(0u64, self.cardinality);
                Column::U64((0..n).map(|_| dist.sample(rng)).collect())
            }
            DType::F64 => {
                // Deliberately the *same* discrete draw as i64/u64, not
                // a continuous `Uniform::new(0.0, cardinality)`. See the
                // `Shape::Uniform` docs: `cardinality` must mean the
                // distinct-key count for every dtype, and holding the
                // logical values fixed across dtypes is what makes
                // dtype a controlled variable in a benchmark.
                let dist = Uniform::new(0u64, self.cardinality);
                Column::F64((0..n).map(|_| dist.sample(rng) as f64).collect())
            }
        })
    }
}

/// Zipfian over ranks `[1, cardinality]`. Mirrors `workload::ZipfI64`
/// (`rand_distr::Zipf` + Xoshiro) and generalizes over the dtype. Note:
/// this is a different algorithm from the legacy CDF `.bin` files, so
/// its bytes are not identical to those.
struct ZipfGen {
    cardinality: u64,
    s: f64,
    dtype: DType,
}

impl ColumnGenerator for ZipfGen {
    fn dtype(&self) -> DType {
        self.dtype
    }

    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        // Validated in `build`; `?` keeps this total.
        let dist = Zipf::new(self.cardinality, self.s)
            .map_err(|e| SketchCoreError::BadParam(format!("zipf: {e}")))?;
        Ok(match self.dtype {
            DType::I64 => Column::I64((0..n).map(|_| dist.sample(rng) as i64).collect()),
            DType::U64 => Column::U64((0..n).map(|_| dist.sample(rng) as u64).collect()),
            DType::F64 => Column::F64((0..n).map(|_| dist.sample(rng)).collect()),
        })
    }
}

/// Monotonically non-decreasing timestamps built by accumulating gaps.
/// The accumulator is an `i128` call-local so a fresh-seeded call is
/// bit-reproducible; overflow past the target type is a hard error.
struct TimestampGen {
    start: i64,
    gap: GapSampler,
    min_gap: u64,
    dtype: DType,
}

impl ColumnGenerator for TimestampGen {
    fn dtype(&self) -> DType {
        self.dtype
    }

    fn generate(&self, n: usize, rng: &mut Xoshiro256PlusPlus) -> Result<Column, SketchCoreError> {
        let overflow = || SketchCoreError::BadParam("monotonic_timestamp: value overflow".into());
        let mut acc: i128 = self.start as i128;
        match self.dtype {
            DType::I64 => {
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    if i > 0 {
                        acc += self.gap.sample_ticks(rng).max(self.min_gap) as i128;
                    }
                    out.push(i64::try_from(acc).map_err(|_| overflow())?);
                }
                Ok(Column::I64(out))
            }
            DType::U64 => {
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    if i > 0 {
                        acc += self.gap.sample_ticks(rng).max(self.min_gap) as i128;
                    }
                    out.push(u64::try_from(acc).map_err(|_| overflow())?);
                }
                Ok(Column::U64(out))
            }
            DType::F64 => Err(SketchCoreError::BadParam(
                "monotonic_timestamp: dtype f64 unsupported".into(),
            )),
        }
    }
}

/// Draws category ids from a fixed domain with a configurable skew.
struct SkewedCategoricalGen {
    categories: Vec<i64>,
    index: WeightedIndex<f64>,
}

impl ColumnGenerator for SkewedCategoricalGen {
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
