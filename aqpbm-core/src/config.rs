//! Sketch-construction parameters.
//!
//! ## Why the family set is open
//!
//! This used to be a closed `enum ParamSet { Hll(..), Kll(..), Cms(..), .. }`,
//! and everything that needed to know about a family matched on it. Adding one
//! family therefore meant editing **131 match arms across six files**: the
//! enum and its `family()`, a `build_*` constructor and an allowed-key list in
//! the sweep parser, a default grid, a CSV column header and a CSV row
//! formatter, and a `family -> statistic` table whose doc comment asked the
//! reader to "keep it in sync" with a field in the dispatch table by hand.
//! Nothing enforced any of it; a missed arm was a runtime `bail!` at best.
//!
//! A benchmark whose whole point is comparing implementations must make adding
//! one obvious. So the family axis is open: [`ParamSet`] carries the family
//! name and its parameters as JSON, and each parameter type declares its own
//! name, its own default grid, and — through serde — its own parsing and its
//! own field names.
//!
//! The record shape is **field-for-field compatible** — still
//! `{"family": "...", "params": {...}}`, and every pre-existing record still
//! deserialises — but it is not byte-identical: the enum serialised the params
//! struct directly, so keys came out in declaration order, while a
//! `serde_json::Value` is a `BTreeMap` and sorts them. `{"rows":5,"cols":2048}`
//! now reads `{"cols":2048,"rows":5}`. In-repo consumers are unaffected
//! (`scripts/merge_passes.py` sorts keys before comparing), but external
//! tooling that diffs or dedups `sketch_config` as a raw string will see every
//! config as new across this boundary.
//!
//! Adding a family is now: one params struct with `#[derive(Serialize,
//! Deserialize)]`, one `impl SketchParams`, and the dispatch row that names
//! it. There is nothing else to keep in sync.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use aqpbm_datagen::SketchError;

/// Construction parameters for one sketch family.
///
/// `deny_unknown_fields` on each implementor is what turns a typo in
/// `--config` into an error naming the offending key, which the hand-written
/// allowed-key lists used to do one family at a time.
pub trait SketchParams: Serialize + DeserializeOwned + Clone + std::fmt::Debug {
    /// The `--sketch` name this parameterises.
    const FAMILY: &'static str;

    /// The grid swept when `--config` is omitted.
    ///
    /// Lives on the params type rather than in a table keyed by family name,
    /// so it cannot drift from the type it configures, and so a new family
    /// arrives with its grid already attached.
    fn default_grid() -> Vec<Self>;
}

/// Family-tagged parameters, type-erased so the set of families stays open.
///
/// Serialises as `{"family": "...", "params": {...}}` — the shape the
/// `sketch_config` field of a v2 record has always had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSet {
    pub family: String,
    pub params: serde_json::Value,
}

impl ParamSet {
    /// Erase a typed params value.
    pub fn of<P: SketchParams>(p: &P) -> Self {
        Self {
            family: P::FAMILY.to_string(),
            params: serde_json::to_value(p).expect("params -> JSON should not fail"),
        }
    }

    /// Recover the typed value.
    ///
    /// Fails if this set belongs to another family, or if the JSON does not
    /// match `P` — which is how an unknown or misspelled `--config` key is
    /// reported, with serde naming the key and listing the valid ones.
    pub fn parse<P: SketchParams>(&self) -> Result<P, SketchError> {
        if self.family != P::FAMILY {
            return Err(SketchError::BadParam(format!(
                "params are for family '{}', not '{}'",
                self.family,
                P::FAMILY
            )));
        }
        serde_json::from_value(self.params.clone())
            .map_err(|e| SketchError::BadParam(format!("{} params: {e}", P::FAMILY)))
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// The whole tagged object, for the record's `sketch_config` field.
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ParamSet -> JSON should not fail")
    }

    /// `(key, value)` pairs of the parameters, ordered by key.
    ///
    /// Lets the CSV writer emit a header and a row for any family without a
    /// per-family column table. Ordering is by key so the header of one file
    /// is stable across runs.
    pub fn fields(&self) -> Vec<(String, String)> {
        let Some(obj) = self.params.as_object() else {
            return Vec::new();
        };
        obj.iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Families declared right here, so these tests exercise the open
    /// axis without `aqpbm-core` knowing any real family. That the
    /// axis can be exercised this way *is* the property under test:
    /// the concrete families live in `aqpbm-cli::params`.
    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FakeParams {
        rows: usize,
        cols: usize,
    }

    impl SketchParams for FakeParams {
        const FAMILY: &'static str = "fake";
        fn default_grid() -> Vec<Self> {
            vec![FakeParams {
                rows: 5,
                cols: 2048,
            }]
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OtherParams {
        lg_k: u8,
    }

    impl SketchParams for OtherParams {
        const FAMILY: &'static str = "other";
        fn default_grid() -> Vec<Self> {
            vec![OtherParams { lg_k: 14 }]
        }
    }

    #[test]
    fn serialises_to_the_documented_bytes() {
        // Pinned exactly, so a future representation change cannot silently
        // alter the record shape the way the enum -> Value move altered key
        // order. Asserting only field *values* would not have caught that.
        let p = ParamSet::of(&FakeParams {
            rows: 5,
            cols: 2048,
        });
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"family":"fake","params":{"cols":2048,"rows":5}}"#
        );
    }

    #[test]
    fn roundtrips_through_json() {
        let p = ParamSet::of(&OtherParams { lg_k: 14 });
        let back: ParamSet = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.parse::<OtherParams>().unwrap().lg_k, 14);
    }

    #[test]
    fn parsing_as_the_wrong_family_fails() {
        let p = ParamSet::of(&OtherParams { lg_k: 14 });
        let err = p.parse::<FakeParams>().unwrap_err().to_string();
        assert!(err.contains("other") && err.contains("fake"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected_by_name() {
        // What the hand-written per-family allowed-key lists used to do.
        let p = ParamSet {
            family: "fake".into(),
            params: serde_json::json!({"rows": 5, "colz": 2048}),
        };
        let err = p.parse::<FakeParams>().unwrap_err().to_string();
        assert!(err.contains("colz"), "error should name the bad key: {err}");
    }

    #[test]
    fn fields_are_ordered_and_stringified() {
        let p = ParamSet::of(&FakeParams {
            rows: 5,
            cols: 2048,
        });
        assert_eq!(
            p.fields(),
            vec![
                ("cols".to_string(), "2048".to_string()),
                ("rows".to_string(), "5".to_string())
            ]
        );
    }
}
