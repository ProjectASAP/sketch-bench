//! One column: its description, the checks that description must pass, and the
//! draw → rule → shift → render pipeline that turns it into a [`ColumnData`].

use rand::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::dist::DataDistribution;
use crate::error::DataGenError;
use crate::rule::{self, MonotonicAcc, RULE_MONOTONIC_INCREASE, RULE_NONE};
use crate::value::{ColumnData, GenValue, StringOpts};

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

    /// Apply this column's rule, shift and `data_type` to a raw draw stream.
    pub(crate) fn render(&self, raw: &[f64]) -> Result<ColumnData, DataGenError> {
        // The one place `data_type` is read. Each arm below is a monomorphic
        // loop, so the tag costs one match per column and none per value.
        match self.data_type.as_str() {
            "i64" => self.render_as::<i64>(raw),
            "u64" => self.render_as::<u64>(raw),
            "f64" => self.render_as::<f64>(raw),
            "string" => self.render_as::<String>(raw),
            other => Err(DataGenError::BadParam(format!(
                "data_type: unknown type '{other}'"
            ))),
        }
    }

    fn render_as<T: GenValue>(&self, raw: &[f64]) -> Result<ColumnData, DataGenError> {
        let cfg = T::cfg(self)?;
        let shift = self.shift.unwrap_or(0.0);
        let mut out: Vec<T> = Vec::with_capacity(raw.len());

        if self.special_rule & RULE_MONOTONIC_INCREASE != 0 {
            // The draws are gaps, and `shift` is where the series begins.
            let mut acc = MonotonicAcc::new(shift);
            for draw in raw {
                out.push(T::render(acc.push(*draw)?, &cfg));
            }
        } else {
            for draw in raw {
                out.push(T::render(draw + shift, &cfg));
            }
        }
        Ok(T::into_column(out))
    }
}
