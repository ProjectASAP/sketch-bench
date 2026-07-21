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

use std::marker::PhantomData;

use crate::error::SketchError;

use super::dist::{Distribution, GapSampler, KeySampler};
use super::{DType, GenValue};

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
fn reject_inexact_f64(dtype: DType, cardinality: u64) -> Result<(), SketchError> {
    const LIMIT: u64 = 1 << 53;
    if dtype == DType::F64 && cardinality > LIMIT {
        return Err(SketchError::BadParam(format!(
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
    },
}

impl Shape {
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
    pub fn build<T: GenValue>(&self) -> Result<Generator<T>, SketchError> {
        match self {
            Shape::Keys { cardinality, dist } => {
                if *cardinality == 0 {
                    return Err(SketchError::BadParam(
                        "keys: cardinality must be > 0".into(),
                    ));
                }
                reject_inexact_f64(T::DTYPE, *cardinality)?;
                Ok(Generator::Keys(KeysGen {
                    sampler: dist.key_sampler(*cardinality)?,
                    _item: PhantomData,
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
                    _item: PhantomData,
                }))
            }
            Shape::Monotonic {
                start,
                gap,
                min_gap,
                unit: _,
            } => {
                if T::DTYPE == DType::F64 {
                    return Err(SketchError::BadParam(
                        "monotonic: dtype f64 unsupported; use i64 or u64".into(),
                    ));
                }
                Ok(Generator::Monotonic(MonotonicGen {
                    gap: gap.gap_sampler()?,
                    min_gap: *min_gap,
                    acc: *start as i128,
                    started: false,
                    _item: PhantomData,
                }))
            }
        }
    }
}

/// A prepared generator: the closed set of structures, one per `Shape`
/// variant. Built by [`Shape::build`], driven by [`GenSpec::generate`].
/// An enum rather than a trait object — the set is closed and
/// crate-private, matching how every other datagen concern
/// ([`Distribution`], [`Column`], [`Shape`]) is modelled.
pub enum Generator<T> {
    Keys(KeysGen<T>),
    Categorical(CategoricalGen<T>),
    Monotonic(MonotonicGen<T>),
}

impl<T: GenValue> Generator<T> {
    /// Produce the next `n` values.
    ///
    /// Takes `&mut self` because a structure may carry state *between*
    /// calls (the monotonic accumulator does). That is what makes
    /// chunked generation identical to one-shot generation: N calls of
    /// `n` and one call of `N*n` consume the same RNG draws and
    /// continue the same series.
    /// Appends to `out` rather than returning a fresh `Vec`, so the chunked
    /// driver reuses one buffer for the whole run instead of allocating per
    /// chunk. `out` is cleared by the caller.
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
pub struct KeysGen<T> {
    sampler: KeySampler,
    _item: PhantomData<T>,
}

impl<T: GenValue> KeysGen<T> {
    fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        out.extend((0..n).map(|_| T::from_draw(self.sampler.sample(rng))));
        Ok(())
    }
}

/// Draws category ids from a fixed domain with a configurable skew.
pub struct CategoricalGen<T> {
    categories: Vec<i64>,
    index: WeightedIndex<f64>,
    _item: PhantomData<T>,
}

impl<T: GenValue> CategoricalGen<T> {
    /// Category ids are authored as `i64` in the spec, so they are narrowed
    /// through [`GenValue::from_acc`] rather than rendered from a draw — an
    /// id is a value the user wrote down, not a sample. `from_acc` is
    /// fallible, which is what makes `dtype: u64` over negative ids an error
    /// rather than a wrap; there is no longer a `Shape::dtype()` pinning this
    /// shape to `i64` in advance.
    fn generate(
        &mut self,
        n: usize,
        rng: &mut Xoshiro256PlusPlus,
        out: &mut Vec<T>,
    ) -> Result<(), SketchError> {
        for _ in 0..n {
            out.push(T::from_acc(
                self.categories[self.index.sample(rng)] as i128,
            )?);
        }
        Ok(())
    }
}

/// Monotonically non-decreasing values built by accumulating gaps.
///
/// The accumulator is an `i128` **field**, not a call-local: the series
/// has to continue across chunk boundaries, so a fresh generator (built
/// once per [`super::GenSpec::generate_into`]) starts at `start` and
/// each subsequent call resumes where the last left off. Overflow past
/// the target type is a hard error.
pub struct MonotonicGen<T> {
    gap: GapSampler,
    min_gap: u64,
    /// Running value; seeded with `start` at build time.
    acc: i128,
    /// Whether any value has been emitted yet. The very first value of
    /// the series is `start` itself, with no gap applied — that must
    /// hold for the series, not for each chunk.
    started: bool,
    _item: PhantomData<T>,
}

impl<T: GenValue> MonotonicGen<T> {
    /// Accumulates in the widest integer and narrows per value. The
    /// intermediate `Vec<i128>` this used to build is gone: it existed only
    /// so the per-dtype narrowing could happen in one trailing match, and
    /// there is no match left.
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
            out.push(T::from_acc(self.acc)?);
        }
        Ok(())
    }
}
