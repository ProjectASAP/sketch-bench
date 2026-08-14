//! The rendering axis: how one processed draw becomes a value of the type a
//! column's `data_type` names, and the [`ColumnData`] a rendered column peels
//! into.
//!
//! `data_type` is read once per column, at the top of generation, and the loop
//! underneath it is monomorphic in `T` — so the run-time tag costs one match per
//! column rather than one per value.

use serde::{Deserialize, Serialize};

use crate::column::ColumnSpec;
use crate::error::DataGenError;

/// One generated column, tagged with the type it holds. The output side of
/// `data_type`: a caller peels the variant it asked for and takes ownership of
/// the values directly.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    Unsigned64(Vec<u64>),
    String(Vec<String>),
}

impl ColumnData {
    /// The `data_type` spelling that produces this variant.
    pub fn kind(&self) -> &'static str {
        match self {
            ColumnData::Int64(_) => "i64",
            ColumnData::Float64(_) => "f64",
            ColumnData::Unsigned64(_) => "u64",
            ColumnData::String(_) => "string",
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ColumnData::Int64(v) => v.len(),
            ColumnData::Float64(v) => v.len(),
            ColumnData::Unsigned64(v) => v.len(),
            ColumnData::String(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Peel by ownership, one method per variant. A mismatch is an error naming both
/// types: it means the description's `data_type` and the consumer's item type
/// disagree, and coercing would run the measurement over values nobody asked for.
macro_rules! peel {
    ($method:ident, $variant:ident, $ty:ty, $name:literal) => {
        impl ColumnData {
            pub fn $method(self) -> Result<Vec<$ty>, DataGenError> {
                match self {
                    ColumnData::$variant(v) => Ok(v),
                    other => Err(DataGenError::TypeMismatch {
                        held: other.kind(),
                        wanted: $name,
                    }),
                }
            }
        }
    };
}

peel!(into_i64, Int64, i64, "i64");
peel!(into_f64, Float64, f64, "f64");
peel!(into_u64, Unsigned64, u64, "u64");
peel!(into_string, String, String, "string");

/// A type a column can be rendered as. Adding one is an impl here plus an arm in
/// [`crate::column::generate_column`], and nothing else.
pub trait GenValue: Clone + std::fmt::Debug + PartialEq + 'static {
    /// Per-type rendering configuration, built once per column. `()` for the
    /// numerics, where rendering is a cast; a string needs an alphabet, a length
    /// rule and the domain its ranks are drawn over.
    type Cfg: Clone;

    /// The `data_type` spelling that selects this type.
    const NAME: &'static str;

    /// Build the configuration from the column's description, validating it
    /// eagerly so a bad `string:` block fails before any value is drawn.
    fn cfg(spec: &ColumnSpec) -> Result<Self::Cfg, DataGenError>;

    /// Render one processed draw. Numeric renders use `as`, which truncates
    /// toward zero and saturates at the type's bounds.
    fn render(v: f64, cfg: &Self::Cfg) -> Self;

    /// Value as `f64` for the column summary, or `None` where it does not apply
    /// — a string's *length* under a field named `min` would be a lie.
    fn stat(&self) -> Option<f64>;

    /// Wrap a rendered column in its [`ColumnData`] variant.
    fn into_column(values: Vec<Self>) -> ColumnData;

    /// Peel a column back out at this type. The inverse of [`Self::into_column`],
    /// and the one place a consumer's item type meets a description's
    /// `data_type`: a mismatch is an error naming both.
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError>;
}

macro_rules! numeric_value {
    ($ty:ty, $name:literal, $variant:ident, $peel:ident) => {
        impl GenValue for $ty {
            type Cfg = ();
            const NAME: &'static str = $name;

            fn cfg(_spec: &ColumnSpec) -> Result<(), DataGenError> {
                Ok(())
            }

            #[inline(always)]
            fn render(v: f64, _cfg: &()) -> Self {
                v as $ty
            }

            #[inline(always)]
            fn stat(&self) -> Option<f64> {
                Some(*self as f64)
            }

            fn into_column(values: Vec<Self>) -> ColumnData {
                ColumnData::$variant(values)
            }

            fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
                column.$peel()
            }
        }
    };
}

numeric_value!(i64, "i64", Int64, into_i64);
numeric_value!(u64, "u64", Unsigned64, into_u64);
numeric_value!(f64, "f64", Float64, into_f64);

/// How a drawn rank becomes a string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringOpts {
    /// Characters strings are built from. Order matters: it is the digit order
    /// of the positional encoding below.
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

/// Validated [`StringOpts`], the prefix width injectivity requires, and the
/// domain floor that turns a draw into a 0-based rank.
#[derive(Debug, Clone)]
pub struct StrCfg {
    alphabet: Vec<char>,
    min_len: usize,
    max_len: usize,
    /// Leading characters positionally encoding the rank — a base-`|alphabet|`
    /// encoding, so two ranks always differ within them. That is what keeps the
    /// rendering **injective**, and the `cardinality` claim honest.
    prefix: usize,
    /// Lowest value the column's distribution draws, subtracted so rank 0 is the
    /// first position of the domain whatever the distribution starts at.
    lower: f64,
}

impl StrCfg {
    pub(crate) fn build(
        opts: &StringOpts,
        cardinality: u64,
        lower: f64,
    ) -> Result<Self, DataGenError> {
        let alphabet: Vec<char> = opts.alphabet.chars().collect();
        if alphabet.len() < 2 {
            return Err(DataGenError::BadParam(
                "string: alphabet needs at least 2 distinct characters".into(),
            ));
        }
        {
            let mut seen: Vec<char> = alphabet.clone();
            seen.sort_unstable();
            seen.dedup();
            if seen.len() != alphabet.len() {
                return Err(DataGenError::BadParam(
                    "string: alphabet has repeated characters, which would collapse distinct keys"
                        .into(),
                ));
            }
        }
        if opts.min_len == 0 {
            return Err(DataGenError::BadParam("string: min_len must be > 0".into()));
        }
        if opts.min_len > opts.max_len {
            return Err(DataGenError::BadParam(format!(
                "string: min_len {} exceeds max_len {}",
                opts.min_len, opts.max_len
            )));
        }
        // Smallest prefix width that can address `cardinality` distinct ranks in
        // this alphabet.
        let base = alphabet.len() as u128;
        let mut prefix = 1usize;
        let mut capacity = base;
        while capacity < cardinality as u128 {
            capacity *= base;
            prefix += 1;
        }
        if prefix > opts.max_len {
            return Err(DataGenError::BadParam(format!(
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
            lower,
        })
    }

    /// Deterministic per-rank scramble. Not an RNG: the length and the filler of
    /// a key must depend on the key alone, so the same rank renders to the same
    /// string every time it is drawn.
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
        // Filler is derived from the rank too, so it carries no information the
        // prefix has not already fixed and cannot make two ranks collide.
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

    fn cfg(spec: &ColumnSpec) -> Result<StrCfg, DataGenError> {
        let domain = spec.distribution.domain().ok_or_else(|| {
            DataGenError::BadParam(format!(
                "string: needs a bounded domain, and {} has none, so its values \
                 cannot be rendered injectively",
                spec.distribution.tag(),
            ))
        })?;
        StrCfg::build(
            spec.string.as_ref().unwrap_or(&StringOpts::default()),
            domain.size,
            domain.lower,
        )
    }

    #[inline]
    fn render(v: f64, cfg: &StrCfg) -> Self {
        cfg.render((v - cfg.lower) as u64)
    }

    fn stat(&self) -> Option<f64> {
        None
    }

    fn into_column(values: Vec<Self>) -> ColumnData {
        ColumnData::String(values)
    }

    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
        column.into_string()
    }
}
