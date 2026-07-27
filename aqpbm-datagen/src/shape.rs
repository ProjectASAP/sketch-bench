//! The structure axis — *what* the generated values mean, orthogonal to the
//! [`Distribution`] saying *how* they are spread. Each variant owns a
//! distribution and the domain it applies over. Adding a structure is a `*Gen`
//! struct, a [`Generator`] variant, and a variant + build arm here.

use rand::distributions::WeightedIndex;
use rand_distr::Distribution as _;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchError;

use super::dist::{Distribution, GapSampler, KeySampler};
use super::GenValue;

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

/// A type that holds integers exactly only up to some limit (`f64`: 2^53)
/// would silently collapse a larger key space onto rounded values.
fn reject_inexact<T: GenValue>(cardinality: u64) -> Result<(), SketchError> {
    if let Some(limit) = T::EXACT_INTEGER_LIMIT {
        if cardinality > limit {
            return Err(SketchError::BadParam(format!(
                "{} cannot hold {cardinality} distinct integers exactly \
                 (limit {limit}); values would round silently",
                T::NAME,
            )));
        }
    }
    Ok(())
}

/// A generation structure: a domain plus the [`Distribution`] drawn over
/// it. `#[serde(tag = "shape")]` makes this the canonical (de)serialized
/// form used by `GenSpec`/`GenMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// A large key space of `cardinality` distinct keys drawn per `dist`
    /// (range-valued: uniform / zipf / constant). The base is per-dist, but the
    /// count is always `cardinality` at every dtype, so `dtype` varies alone.
    Keys {
        cardinality: u64,
        #[serde(default = "default_dist")]
        dist: Distribution,
    },
    /// A finite domain of category ids, drawn per `dist` (uniform / zipf
    /// / explicit weights). Always emits `i64`.
    Categorical {
        categories: Vec<i64>,
        #[serde(default = "default_dist")]
        dist: Distribution,
    },
    /// A monotonically non-decreasing series (e.g. event timestamps): each value
    /// is the previous plus a non-negative gap from `gap`. `min_gap` floors it —
    /// 1 for strictly increasing, 0 to allow duplicates.
    Monotonic {
        #[serde(default)]
        start: i64,
        #[serde(default)]
        unit: TimeUnit,
        gap: Distribution,
        #[serde(default = "default_min_gap")]
        min_gap: u64,
    },
}

impl Shape {
    /// How many distinct values this shape can produce, when bounded. `string`
    /// needs it to size the injective prefix, and `Monotonic` has no answer —
    /// which is why a monotonic string series is refused, not approximated.
    pub fn domain_size(&self) -> Option<u64> {
        match self {
            Shape::Keys { cardinality, .. } => Some(*cardinality),
            Shape::Categorical { categories, .. } => Some(categories.len() as u64),
            Shape::Monotonic { .. } => None,
        }
    }

    /// The label used in reports and CLI output — **not** a clean structural
    /// tag: `Keys` returns the *distribution* name so `shape=="zipf"` filters
    /// keep working. Use the enum variant to identify the structure axis.
    pub fn report_label(&self) -> &'static str {
        match self {
            Shape::Keys { dist, .. } => dist.tag(),
            Shape::Categorical { .. } => "skewed_categorical",
            Shape::Monotonic { .. } => "monotonic_timestamp",
        }
    }

    /// Build the concrete generator, validating parameters eagerly so
    /// bad specs fail before any allocation.
    pub fn build<T: GenValue>(&self, cfg: T::Cfg) -> Result<Generator<T>, SketchError> {
        match self {
            Shape::Keys { cardinality, dist } => {
                if *cardinality == 0 {
                    return Err(SketchError::BadParam(
                        "keys: cardinality must be > 0".into(),
                    ));
                }
                reject_inexact::<T>(*cardinality)?;
                Ok(Generator::Keys(KeysGen {
                    sampler: dist.key_sampler(*cardinality)?,
                    cfg,
                }))
            }
            Shape::Categorical { categories, dist } => {
                if categories.is_empty() {
                    return Err(SketchError::BadParam(
                        "categorical: categories must be non-empty".into(),
                    ));
                }
                let weights = dist.weights(categories.len())?;
                let index = WeightedIndex::new(&weights)
                    .map_err(|e| SketchError::BadParam(format!("weighted index: {e}")))?;
                Ok(Generator::Categorical(CategoricalGen {
                    categories: categories.clone(),
                    index,
                    cfg,
                }))
            }
            Shape::Monotonic {
                start,
                gap,
                min_gap,
                unit: _,
            } => {
                if !T::SUPPORTS_MONOTONIC {
                    return Err(SketchError::BadParam(format!(
                        "monotonic: {} unsupported; use i64 or u64",
                        T::NAME,
                    )));
                }
                Ok(Generator::Monotonic(MonotonicGen {
                    gap: gap.gap_sampler()?,
                    min_gap: *min_gap,
                    acc: *start as i128,
                    started: false,
                    cfg,
                }))
            }
        }
    }
}

/// A prepared generator: the closed set of structures, one per `Shape` variant.
/// Built by [`Shape::build`]. An enum rather than a trait object, matching how
/// the other datagen axes are modelled.
pub enum Generator<T: GenValue> {
    Keys(KeysGen<T>),
    Categorical(CategoricalGen<T>),
    Monotonic(MonotonicGen<T>),
}

impl<T: GenValue> Generator<T> {
    /// Produce the next `n` values into `out`, which the caller clears so one
    /// buffer serves the run. `&mut self` because a structure may carry state
    /// between calls — what makes N calls of `n` match one call of `N*n`.
    pub fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        match self {
            Generator::Keys(g) => g.generate(n, rng, out),
            Generator::Categorical(g) => g.generate(n, rng, out),
            Generator::Monotonic(g) => g.generate(n, rng, out),
        }
    }
}

/// Keys drawn over the sampler's domain, emitted in the requested
/// physical dtype. Every dtype casts the *same* `u64` draw, so a fixed
/// `(shape, size, seed)` is dtype-invariant in its logical values.
pub struct KeysGen<T: GenValue> {
    sampler: KeySampler,
    cfg: T::Cfg,
}

impl<T: GenValue> KeysGen<T> {
    fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        out.extend((0..n).map(|_| T::from_draw(self.sampler.sample(rng), &self.cfg)));
        Ok(())
    }
}

/// Draws category ids from a fixed domain with a configurable skew.
pub struct CategoricalGen<T: GenValue> {
    categories: Vec<i64>,
    index: WeightedIndex<f64>,
    cfg: T::Cfg,
}

impl<T: GenValue> CategoricalGen<T> {
    /// Category ids are authored as `i64`, so they narrow through
    /// [`GenValue::from_acc`] rather than rendering from a draw. Its fallibility
    /// makes `u64` over a negative id an error rather than a wrap.
    fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        for _ in 0..n {
            out.push(T::from_acc(
                self.categories[self.index.sample(rng)] as i128,
                &self.cfg,
            )?);
        }
        Ok(())
    }
}

/// Monotonically non-decreasing values built by accumulating gaps. The
/// accumulator is an `i128` **field**, not a call-local, so the series continues
/// across chunk boundaries. Overflow past the target type is a hard error.
pub struct MonotonicGen<T: GenValue> {
    gap: GapSampler,
    min_gap: u64,
    /// Running value; seeded with `start` at build time.
    acc: i128,
    /// Whether any value has been emitted yet. The very first value of
    /// the series is `start` itself, with no gap applied — that must
    /// hold for the series, not for each chunk.
    started: bool,
    cfg: T::Cfg,
}

impl<T: GenValue> MonotonicGen<T> {
    /// Accumulates in the widest integer and narrows per value.
    fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        for _ in 0..n {
            if self.started {
                self.acc += self.gap.sample_ticks(rng).max(self.min_gap) as i128;
            } else {
                self.started = true;
            }
            out.push(T::from_acc(self.acc, &self.cfg)?);
        }
        Ok(())
    }
}
