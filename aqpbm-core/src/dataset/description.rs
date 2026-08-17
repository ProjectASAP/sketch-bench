//! The report-facing descriptor: what a run's dataset *was*, as it appears in
//! every JSONL record.
//!
//! Deliberately not a [`TableDescription`]. It has to describe datasets that
//! have no spec at all — a file replay carries a path and nothing else — and it
//! is a wire format with producers outside this crate, so its flat fields are
//! frozen. [`DatasetDescription::spec`] is the escape hatch for everything the
//! flat fields cannot hold.

use serde::{Deserialize, Serialize};

use aqpbm_datagen::{DataDistribution, TableDescription};

/// Human-friendly description of a dataset — serialised into
/// every report so a JSONL record can be re-run without
/// external metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetDescription {
    pub shape: String, // "uniform" | "zipf" | "normal" | "columns" | "file"
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zipf_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Full `datagen` description, when the flat fields above cannot express it
    /// (several columns, a shift, a special rule, string options, …). Absent for
    /// a plain single `uniform` / `zipf` column and for `file`, whose flat
    /// fields already round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
}

impl DatasetDescription {
    /// Projection of a [`TableDescription`] into the report-facing descriptor.
    /// Lives here, not on the description, because it asks which of *this*
    /// type's fields can hold one — on the description the generator would be
    /// citing the report schema.
    pub fn from_spec(spec: &TableDescription) -> Self {
        let lead = spec.column_spec.first();
        let multi = spec.column_spec.len() > 1;

        let shape = match (multi, lead) {
            (true, _) => "columns".to_string(),
            (false, Some(c)) => c.distribution.tag().to_string(),
            (false, None) => "empty".to_string(),
        };
        let (cardinality, zipf_s) = match lead {
            Some(c) if !multi => (
                c.distribution.domain().map(|d| d.size),
                match &c.distribution {
                    DataDistribution::Zipf(z) => Some(z.skewness),
                    _ => None,
                },
            ),
            _ => (None, None),
        };

        DatasetDescription {
            shape,
            size: spec.row_num as usize,
            cardinality,
            zipf_s,
            source_path: None,
            seed: lead.map(|c| c.distribution.seed()),
            spec: if fits_legacy_description(spec) {
                None
            } else {
                serde_json::to_value(spec).ok()
            },
        }
    }
}

/// Whether the flat `cardinality` / `zipf_s` fields fully describe `spec`.
/// True only for one plain column drawn uniform or zipf; everything else is
/// lossy there and is the one condition under which
/// [`DatasetDescription::spec`] is set.
///
/// Anything that changes the values without changing those two fields has to
/// force the escape hatch, or two different datasets would share a group key
/// and anything pooling by dataset would average them together.
fn fits_legacy_description(spec: &TableDescription) -> bool {
    let [column] = spec.column_spec.as_slice() else {
        return false;
    };
    column.string.is_none()
        && column.shift.is_none()
        && column.special_rule == aqpbm_datagen::RULE_NONE
        && matches!(
            column.distribution,
            DataDistribution::Uniform(_) | DataDistribution::Zipf(_)
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::zipf_column;
    use aqpbm_datagen::{StringOpts, RULE_MONOTONIC_INCREASE};

    /// Key length is the dominant cost on a hashing insert path, so two runs
    /// at different lengths are two measurements. They must not share a
    /// descriptor, or anything grouping by dataset averages them together.
    #[test]
    fn string_options_reach_the_descriptor() {
        let base = zipf_column(64, 1.0, 1, "string");
        let with_opts = |min_len, max_len| {
            let mut c = base.clone();
            c.string = Some(StringOpts {
                alphabet: "ab".to_string(),
                min_len,
                max_len,
            });
            TableDescription::single("key", c, 32)
        };

        // A plain numeric column keeps the descriptor a record written before
        // the string axis existed would have had.
        let plain = DatasetDescription::from_spec(&TableDescription::single(
            "key",
            zipf_column(64, 1.0, 1, "i64"),
            32,
        ));
        assert!(plain.spec.is_none(), "{plain:?}");

        let short = DatasetDescription::from_spec(&with_opts(4, 4));
        let long = DatasetDescription::from_spec(&with_opts(16, 16));
        assert!(short.spec.is_some());
        assert_ne!(
            serde_json::to_string(&short).unwrap(),
            serde_json::to_string(&long).unwrap(),
            "two key lengths must not share a group key"
        );
    }

    /// Anything that moves the values without moving `cardinality`/`zipf_s` has
    /// to force the full description into the record, or two different
    /// datasets pool as one.
    #[test]
    fn a_shift_or_a_rule_forces_the_full_description() {
        let mut shifted = zipf_column(64, 1.0, 1, "i64");
        shifted.shift = Some(15_000.0);
        let d = DatasetDescription::from_spec(&TableDescription::single("key", shifted, 32));
        assert!(d.spec.is_some());

        let mut ruled = zipf_column(64, 1.0, 1, "i64");
        ruled.special_rule = RULE_MONOTONIC_INCREASE;
        let d = DatasetDescription::from_spec(&TableDescription::single("key", ruled, 32));
        assert!(d.spec.is_some());
    }
}
