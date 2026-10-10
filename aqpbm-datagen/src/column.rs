//! One column: its description, the checks that description must pass, and the
//! draw → rule → shift → render pipeline that turns it into a [`ColumnData`].

use rand::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::dist::DataDistribution;
use crate::error::DataGenError;
use crate::rule::{self, MonotonicAcc, RULE_MONOTONIC_INCREASE, RULE_NONE};
use crate::value::{ColumnData, StrCfg, StringOpts};

/// The `data_type` spellings this build renders.
pub const DATA_TYPES: [&str; 4] = ["i64", "u64", "f64", "string"];

/// One column of a [`crate::TableDescription`].
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub distribution: DataDistribution,
    /// Added to every value, for a column that should not start where its
    /// distribution naturally does. Under the monotonic rule it is the value the
    /// series starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<f64>,
    /// A restatement of the distribution's domain size. Optional, and checked
    /// against the distribution rather than overriding it: two ways of saying
    /// the same thing that disagree are a description error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    /// Bit mask; see [`crate::rule`]. `0` means no special rules.
    #[serde(default)]
    pub special_rule: u32,
    /// Which [`ColumnData`] variant this column renders into.
    pub data_type: String,
    /// Rendering options for `data_type: string`. Absent means the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string: Option<StringOpts>,
    /// Index of an earlier `string` column this one nests under, for
    /// hierarchical labels. A value is the parent row's value, a `.`, and a
    /// child index in `[0, fan_out)` drawn from this column's distribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_of: Option<usize>,
    /// How many children each parent value has. Required with `child_of`, and
    /// checked against the distribution's domain like `cardinality` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<u64>,
    /// Index of an earlier `string` column whose values scale this one. Each
    /// distinct label value gets a fixed factor in `scale_range`, and every
    /// row's value is multiplied by its label's factor. `f64` columns only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_by: Option<usize>,
    /// `[lo, hi]` with `0 < lo <= hi`: the factors are log-uniform in it.
    /// Required with `scale_by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_range: Option<[f64; 2]>,
}

impl ColumnSpec {
    /// Every check that can be made without drawing. Called for each column
    /// before the table generates anything, so a bad description costs no
    /// allocation and the error names the column that carried it.
    pub fn validate(&self) -> Result<(), DataGenError> {
        rule::validate(self.special_rule)?;

        if !DATA_TYPES.contains(&self.data_type.as_str()) {
            return Err(DataGenError::BadParam(format!(
                "data_type: unknown type '{}'; expected one of {}",
                self.data_type,
                DATA_TYPES.join(", "),
            )));
        }

        match (self.child_of, self.fan_out) {
            (None, None) => {}
            (Some(_), None) => {
                return Err(DataGenError::BadParam(
                    "child_of: needs fan_out, the number of children per parent value".into(),
                ))
            }
            (None, Some(_)) => {
                return Err(DataGenError::BadParam(
                    "fan_out: applies only to a child_of column".into(),
                ))
            }
            (Some(_), Some(fan_out)) => self.validate_child(fan_out)?,
        }

        match (self.scale_by, self.scale_range) {
            (None, None) => {}
            (Some(_), None) => {
                return Err(DataGenError::BadParam(
                    "scale_by: needs scale_range, the [lo, hi] the factors are drawn from".into(),
                ))
            }
            (None, Some(_)) => {
                return Err(DataGenError::BadParam(
                    "scale_range: applies only to a scale_by column".into(),
                ))
            }
            (Some(_), Some([lo, hi])) => {
                // An integer column would round the scaled value back down.
                if self.data_type != "f64" {
                    return Err(DataGenError::BadParam(format!(
                        "scale_by: a scaled value is fractional, so data_type has to be \
                         `f64`, not '{}'",
                        self.data_type
                    )));
                }
                if !(lo > 0.0 && hi >= lo && hi.is_finite()) {
                    return Err(DataGenError::BadParam(format!(
                        "scale_range [{lo}, {hi}] needs 0 < lo <= hi"
                    )));
                }
            }
        }

        // `cardinality` restates the domain. Silently preferring one over the
        // other would let a report cite a key space the column never had.
        if let Some(claimed) = self.cardinality {
            match self.distribution.domain() {
                None => {
                    return Err(DataGenError::BadParam(format!(
                        "cardinality {claimed} was given, but {} is unbounded and has no \
                         domain to compare it against; drop the field",
                        self.distribution.tag(),
                    )))
                }
                Some(domain) if domain.size != claimed => {
                    return Err(DataGenError::BadParam(format!(
                        "cardinality {claimed} disagrees with the {} domain, which holds {}",
                        self.distribution.tag(),
                        domain.size,
                    )))
                }
                Some(_) => {}
            }
        }

        // A string is rendered from its rank in the domain, so anything that
        // moves a value off that rank has no meaning here. Refused rather than
        // ignored: a spec that set them would otherwise read as if they applied.
        if self.data_type == "string" {
            if self.shift.is_some() {
                return Err(DataGenError::BadParam(
                    "string: shift moves a value off the rank it is rendered from; \
                     drop it, or render this column as a numeric type"
                        .into(),
                ));
            }
            if self.special_rule != RULE_NONE {
                return Err(DataGenError::BadParam(
                    "string: special_rule applies to the numeric series, not to the \
                     rank a string is rendered from"
                        .into(),
                ));
            }
        }

        Ok(())
    }

    /// The checks a `child_of` column adds. Its index of the parent is checked
    /// by the table, which knows the other columns.
    fn validate_child(&self, fan_out: u64) -> Result<(), DataGenError> {
        if self.data_type != "string" {
            return Err(DataGenError::BadParam(format!(
                "child_of: a child value is text, so data_type has to be `string`, not '{}'",
                self.data_type
            )));
        }
        if fan_out == 0 {
            return Err(DataGenError::BadParam("fan_out must be > 0".into()));
        }
        // A uniform draw is continuous; only whole bounds keep its index in
        // `[0, fan_out)` (`uniform{0.0, 2.9}` would draw 0, 1 and 2).
        if let DataDistribution::Uniform(p) = &self.distribution {
            if p.lower_bound.fract() != 0.0 || p.upper_bound.fract() != 0.0 {
                return Err(DataGenError::BadParam(format!(
                    "child_of: uniform bounds must be whole numbers, got [{}, {})",
                    p.lower_bound, p.upper_bound
                )));
            }
        }
        match self.distribution.domain() {
            Some(domain) if domain.size == fan_out => {}
            Some(domain) => {
                return Err(DataGenError::BadParam(format!(
                    "fan_out {fan_out} disagrees with the {} domain, which holds {}; the \
                     distribution draws the child index",
                    self.distribution.tag(),
                    domain.size,
                )))
            }
            None => {
                return Err(DataGenError::BadParam(format!(
                    "child_of: the child index needs a bounded domain, and {} has none",
                    self.distribution.tag(),
                )))
            }
        }
        // A child column holds parent cardinality × fan_out values and is not
        // rendered from a rank, so neither field would mean what it says.
        if self.cardinality.is_some() {
            return Err(DataGenError::BadParam(
                "child_of: cardinality would restate fan_out, not the column's distinct \
                 count (parent cardinality × fan_out); drop it"
                    .into(),
            ));
        }
        if self.string.is_some() {
            return Err(DataGenError::BadParam(
                "child_of: a child value is its parent's plus an index, so a `string:` \
                 block has nothing to render; drop it"
                    .into(),
            ));
        }
        Ok(())
    }

    /// The raw draw stream: `row_num` values straight from the distribution,
    /// before any rule, shift or rendering. Split out because a
    /// `column_connected` group draws this once and every member column renders
    /// its own values from it.
    pub(crate) fn draw(&self, row_num: usize) -> Result<Vec<f64>, DataGenError> {
        let sampler = self.distribution.sampler()?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(self.distribution.seed());
        Ok((0..row_num).map(|_| sampler.sample(&mut rng)).collect())
    }

    /// The processed series: this column's rule and shift applied to the raw
    /// draws, one pass and no intermediate buffer. Fallible per value, because
    /// the monotonic accumulator can run past the range it counts in.
    fn processed<'a>(
        &self,
        raw: &'a [f64],
    ) -> impl Iterator<Item = Result<f64, DataGenError>> + 'a {
        let shift = self.shift.unwrap_or(0.0);
        let monotonic = self.special_rule & RULE_MONOTONIC_INCREASE != 0;
        // The draws are gaps under the monotonic rule, and `shift` is where the
        // series begins. The accumulator is captured, so it carries across the
        // whole column rather than restarting.
        let mut acc = MonotonicAcc::new(shift);
        raw.iter().map(move |draw| {
            if monotonic {
                acc.push(*draw)
            } else {
                Ok(draw + shift)
            }
        })
    }

    /// Apply this column's rule, shift and `data_type` to a raw draw stream.
    /// The one place `data_type` is read: one match per column, none per value,
    /// each arm its own loop. Numeric arms use `as` — truncating, saturating.
    pub(crate) fn render(&self, raw: &[f64]) -> Result<ColumnData, DataGenError> {
        match self.data_type.as_str() {
            "i64" => Ok(ColumnData::Int64(
                self.processed(raw)
                    .map(|v| v.map(|v| v as i64))
                    .collect::<Result<_, _>>()?,
            )),
            "u64" => Ok(ColumnData::Unsigned64(
                self.processed(raw)
                    .map(|v| v.map(|v| v as u64))
                    .collect::<Result<_, _>>()?,
            )),
            "f64" => Ok(ColumnData::Float64(
                self.processed(raw).collect::<Result<_, _>>()?,
            )),
            "string" => {
                // Built once, before the loop: it validates the `string:` block
                // and sizes the injective prefix from the column's domain.
                let cfg = StrCfg::for_column(self)?;
                Ok(ColumnData::String(
                    self.processed(raw)
                        .map(|v| v.map(|v| cfg.render_draw(v)))
                        .collect::<Result<_, _>>()?,
                ))
            }
            other => Err(DataGenError::BadParam(format!(
                "data_type: unknown type '{other}'"
            ))),
        }
    }

    /// The fixed factor of one `scale_by` label value: a SplitMix64 hash of
    /// the value's FNV-1a and this column's seed, as `u` in `[0, 1)`, mapped to
    /// `lo·(hi/lo)^u`.
    pub(crate) fn scale_factor(&self, label: &str) -> f64 {
        let [lo, hi] = self.scale_range.expect("scale_range was validated above");
        let fnv = label.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3)
        });
        let x = crate::table::mix_connected_bits(fnv ^ self.distribution.seed());
        let u = (x >> 11) as f64 / (1u64 << 53) as f64;
        lo * (hi / lo).powf(u)
    }

    /// Render a `child_of` column: each row's parent value, a `.`, and the
    /// draw's 0-based rank in the domain as the child index.
    pub(crate) fn render_child(
        &self,
        raw: &[f64],
        parents: &[String],
    ) -> Result<ColumnData, DataGenError> {
        let domain = self
            .distribution
            .domain()
            .expect("child domains were validated above");
        Ok(ColumnData::String(
            raw.iter()
                .zip(parents)
                .map(|(draw, parent)| format!("{parent}.{}", (draw - domain.lower) as u64))
                .collect(),
        ))
    }
}
