//! The rendering axis: the [`ColumnData`] a generated column comes back as, how
//! a caller peels one, and how a drawn rank becomes a string. Rendering itself is
//! four match arms in `ColumnSpec::render`, where `data_type` is read once.

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

/// Peel by ownership or read by borrow, one method per variant. A mismatch is an error naming both
/// types: it means the description's `data_type` and the consumer's item type
/// disagree, and coercing would run the measurement over values nobody asked for.
impl ColumnData {
    pub fn into_i64(self) -> Result<Vec<i64>, DataGenError> {
        match self {
            ColumnData::Int64(v) => Ok(v),
            other => Err(other.mismatch("i64")),
        }
    }

    pub fn into_u64(self) -> Result<Vec<u64>, DataGenError> {
        match self {
            ColumnData::Unsigned64(v) => Ok(v),
            other => Err(other.mismatch("u64")),
        }
    }

    pub fn into_f64(self) -> Result<Vec<f64>, DataGenError> {
        match self {
            ColumnData::Float64(v) => Ok(v),
            other => Err(other.mismatch("f64")),
        }
    }

    pub fn into_string(self) -> Result<Vec<String>, DataGenError> {
        match self {
            ColumnData::String(v) => Ok(v),
            other => Err(other.mismatch("string")),
        }
    }

    pub fn as_i64(&self) -> Result<&[i64], DataGenError> {
        match self {
            ColumnData::Int64(v) => Ok(v),
            other => Err(other.mismatch("i64")),
        }
    }

    pub fn as_u64(&self) -> Result<&[u64], DataGenError> {
        match self {
            ColumnData::Unsigned64(v) => Ok(v),
            other => Err(other.mismatch("u64")),
        }
    }

    pub fn as_f64(&self) -> Result<&[f64], DataGenError> {
        match self {
            ColumnData::Float64(v) => Ok(v),
            other => Err(other.mismatch("f64")),
        }
    }

    pub fn as_string(&self) -> Result<&[String], DataGenError> {
        match self {
            ColumnData::String(v) => Ok(v),
            other => Err(other.mismatch("string")),
        }
    }

    fn mismatch(&self, wanted: &'static str) -> DataGenError {
        DataGenError::TypeMismatch {
            held: self.kind(),
            wanted,
        }
    }
}

/// The item type a caller reads a column at. A *consumer's* trait, not the
/// generator's: what a downstream crate cannot write for itself is the pair
/// below — which `data_type` names it, and how to get its own `Vec<T>` back.
pub trait ColumnItem: Clone + std::fmt::Debug + PartialEq + 'static {
    /// The `data_type` spelling that selects this type.
    const NAME: &'static str;

    /// Peel a column at this type. The one place a consumer's item type meets a
    /// description's `data_type`: a mismatch is an error naming both.
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError>;

    fn column_slice(column: &ColumnData) -> Result<&[Self], DataGenError>;
}

impl ColumnItem for i64 {
    const NAME: &'static str = "i64";
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
        column.into_i64()
    }
    fn column_slice(column: &ColumnData) -> Result<&[Self], DataGenError> {
        column.as_i64()
    }
}

impl ColumnItem for u64 {
    const NAME: &'static str = "u64";
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
        column.into_u64()
    }
    fn column_slice(column: &ColumnData) -> Result<&[Self], DataGenError> {
        column.as_u64()
    }
}

impl ColumnItem for f64 {
    const NAME: &'static str = "f64";
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
        column.into_f64()
    }
    fn column_slice(column: &ColumnData) -> Result<&[Self], DataGenError> {
        column.as_f64()
    }
}

impl ColumnItem for String {
    const NAME: &'static str = "string";
    fn from_column(column: ColumnData) -> Result<Vec<Self>, DataGenError> {
        column.into_string()
    }
    fn column_slice(column: &ColumnData) -> Result<&[Self], DataGenError> {
        column.as_string()
    }
}

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
    fn build(opts: &StringOpts, cardinality: u64, lower: f64) -> Result<Self, DataGenError> {
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

    /// Build the rendering config for a column, validating it eagerly so a bad
    /// `string:` block fails before any value is drawn.
    pub(crate) fn for_column(spec: &ColumnSpec) -> Result<Self, DataGenError> {
        let domain = spec.distribution.domain().ok_or_else(|| {
            DataGenError::BadParam(format!(
                "string: needs a bounded domain, and {} has none, so its values \
                 cannot be rendered injectively",
                spec.distribution.tag(),
            ))
        })?;
        Self::build(
            spec.string.as_ref().unwrap_or(&StringOpts::default()),
            domain.size,
            domain.lower,
        )
    }

    /// Render one draw, by its 0-based rank in the column's domain.
    #[inline]
    pub(crate) fn render_draw(&self, v: f64) -> String {
        self.render((v - self.lower) as u64)
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
