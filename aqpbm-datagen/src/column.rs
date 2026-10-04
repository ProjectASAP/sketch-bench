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

/// One column of a [`crate::table::TableDescription`].
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{column, normal, one, uniform, zipf};

    /// The example the design doc gives for `shift`: a Zipf column that should
    /// live in the 15000s rather than starting at its natural rank 1.
    #[test]
    fn shift_moves_a_column_off_its_natural_base() {
        let mut spec = column(zipf(5000, 1.1, 4), "i64");
        spec.shift = Some(15000.0);
        let t = one(spec, 2000).generate().unwrap();
        for v in t.into_column(0).unwrap().into_i64().unwrap() {
            assert!((15001..=20000).contains(&v), "{v} outside 15001..=20000");
        }
    }

    #[test]
    fn shift_applies_to_normal_too() {
        let mut spec = column(normal(0.0, 1.0, 5), "f64");
        spec.shift = Some(1000.0);
        let t = one(spec, 2000).generate().unwrap();
        let values = t.into_column(0).unwrap().into_f64().unwrap();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!((mean - 1000.0).abs() < 0.2, "mean {mean} not near 1000");
    }

    /// `data_type` picks the variant, and nothing else about the description
    /// changes — the same draws land in a different container.
    #[test]
    fn data_type_selects_the_variant() {
        for (ty, kind) in [("i64", "i64"), ("u64", "u64"), ("f64", "f64")] {
            let t = one(column(zipf(64, 1.0, 9), ty), 100).generate().unwrap();
            assert_eq!(t.data[0].kind(), kind);
        }
    }

    #[test]
    fn an_unknown_data_type_is_refused_by_name() {
        let err = one(column(zipf(64, 1.0, 9), "i32"), 10)
            .generate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("i32"), "{err}");
    }

    // ---------- the special_rule mask ----------

    #[test]
    fn the_monotonic_rule_never_decreases_and_starts_at_shift() {
        let mut spec = column(uniform(0.0, 10.0, 21), "i64");
        spec.special_rule = RULE_MONOTONIC_INCREASE;
        spec.shift = Some(1_700_000_000_000.0);
        let t = one(spec, 5000).generate().unwrap();
        let values = t.into_column(0).unwrap().into_i64().unwrap();
        assert_eq!(values[0], 1_700_000_000_000);
        assert!(values.windows(2).all(|w| w[1] >= w[0]));
        assert!(values[4999] > values[0], "the series never advanced");
    }

    #[test]
    fn an_unknown_rule_bit_is_refused() {
        let mut spec = column(zipf(64, 1.0, 9), "i64");
        spec.special_rule = 0b1010;
        let err = one(spec, 10).generate().unwrap_err().to_string();
        assert!(err.contains("unknown bit"), "{err}");
    }

    #[test]
    fn a_string_column_refuses_shift_and_rules() {
        let mut spec = column(zipf(64, 1.0, 9), "string");
        spec.shift = Some(1.0);
        assert!(one(spec.clone(), 10).generate().is_err());

        spec.shift = None;
        spec.special_rule = RULE_MONOTONIC_INCREASE;
        assert!(one(spec, 10).generate().is_err());
    }

    // ---------- cardinality as a restatement ----------

    #[test]
    fn a_cardinality_that_agrees_with_the_domain_is_accepted() {
        let mut spec = column(zipf(256, 1.1, 9), "i64");
        spec.cardinality = Some(256);
        assert!(one(spec, 100).generate().is_ok());
    }

    #[test]
    fn a_cardinality_that_disagrees_names_both_numbers() {
        let mut spec = column(zipf(256, 1.1, 9), "i64");
        spec.cardinality = Some(1000);
        let err = one(spec, 100).generate().unwrap_err().to_string();
        assert!(err.contains("1000") && err.contains("256"), "{err}");
    }

    #[test]
    fn a_cardinality_over_an_unbounded_distribution_is_refused() {
        let mut spec = column(normal(0.0, 1.0, 9), "f64");
        spec.cardinality = Some(100);
        let err = one(spec, 100).generate().unwrap_err().to_string();
        assert!(err.contains("unbounded"), "{err}");
    }
}
