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
    ///
    /// The one place `data_type` is read: the tag costs one match per column and
    /// none per value, and each arm is its own monomorphic loop. The numeric
    /// arms use `as`, which truncates toward zero and saturates at the type's
    /// bounds.
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
}
