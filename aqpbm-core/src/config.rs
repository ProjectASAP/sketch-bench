//! Sketch-construction parameters.
//!
//! ## Why the family set is open
//!
//! This used to be a closed `enum ParamSet { Hll(..), Kll(..), Cms(..), .. }`
//! that everything matched on, so adding one family meant editing **131 match
//! arms across six files** — the enum and its `family()`, a `build_*`
//! constructor and an allowed-key list in the config parser, a CSV header and
//! row formatter, and a `family -> statistic` table whose doc comment asked
//! the reader to keep it in sync by hand. Nothing enforced any of it; a missed
//! arm was a runtime `bail!` at best.
//!
//! A benchmark whose point is comparing implementations must make adding one
//! obvious, so the family axis is open: [`ParamSet`] carries the family name
//! and its parameters as JSON, and each parameter type declares its own name,
//! its own canonical config, and — through serde — its own parsing and field
//! names.
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
//! Deserialize)]`, one `impl SketchParams`, and the catalog row that names
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

    /// One representative, buildable config for the family.
    ///
    /// Lives on the params type rather than in a table keyed by family name,
    /// so it cannot drift from the type it configures, and so a new family
    /// arrives with a valid config already attached. Not a sweep: it is the
    /// single point the dtype-acceptance tests build each impl from.
    fn canonical() -> Self;
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

    /// The parameterless point for a family — what `--config` defaults to,
    /// and the config an impl with no tunable knobs (e.g. an exact DataFrame
    /// engine) builds from. An impl that needs parameters rejects it at
    /// construction, naming the field it is missing.
    pub fn empty(family: &str) -> Self {
        Self {
            family: family.to_string(),
            params: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Parse one `--config` point: `'k1=v1 k2=v2'`, exactly one value per key.
    ///
    /// Whitespace separates keys, `=` separates a key from its value. This is
    /// **one** configuration, not a grid: a key given a comma list of values
    /// is an error, because a single invocation measures a single cell.
    /// Running a series is the caller's job — invoke once per point.
    ///
    /// Values are typed by content: `5` becomes a JSON number, `1.1` a float,
    /// `true` a bool, anything else a string. That is what lets one parser
    /// that knows no family feed differently-shaped params structs — serde
    /// does the final coercion when the caller calls [`ParamSet::parse`], and
    /// `deny_unknown_fields` rejects a mismatch by field name.
    ///
    /// Deliberately **not** validated here beyond syntax: this function cannot
    /// know whether `family` exists or whether the keys belong to it, because
    /// `aqpbm-core` names no family. A caller that owns the family registry
    /// should check membership before calling, and type-check the returned set
    /// with [`ParamSet::parse`] before measuring anything.
    pub fn single(family: &str, spec: &str) -> Result<ParamSet, SketchError> {
        let axes = parse_axes(spec)?;
        let mut params = serde_json::Map::new();
        for (key, values) in axes {
            if values.len() > 1 {
                return Err(SketchError::BadParam(format!(
                    "config key '{key}' lists {} values; --config takes one value \
                     per key (a single cell). Invoke once per point to measure a series.",
                    values.len()
                )));
            }
            params.insert(key, typed(&values[0]));
        }
        Ok(ParamSet {
            family: family.to_string(),
            params: serde_json::Value::Object(params),
        })
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

/// `"5"` → number, `"1.1"` → float, `"true"` → bool, else string.
fn typed(raw: &str) -> serde_json::Value {
    if let Ok(i) = raw.parse::<i64>() {
        return serde_json::Value::from(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return serde_json::Value::from(f);
    }
    if let Ok(b) = raw.parse::<bool>() {
        return serde_json::Value::from(b);
    }
    serde_json::Value::from(raw)
}

/// Split `'k=v1,v2 k2=v3'` into its axes, keeping their order so the
/// expansion is deterministic.
fn parse_axes(spec: &str) -> Result<Vec<(String, Vec<String>)>, SketchError> {
    let mut out = Vec::new();
    for tok in spec.split_whitespace() {
        let (key, vals) = tok
            .split_once('=')
            .ok_or_else(|| SketchError::BadParam(format!("config token missing '=': {tok}")))?;
        let vals: Vec<String> = vals.split(',').map(|s| s.trim().to_string()).collect();
        if vals.is_empty() || vals.iter().any(|v| v.is_empty()) {
            return Err(SketchError::BadParam(format!(
                "config key '{key}' has an empty value list"
            )));
        }
        out.push((key.trim().to_string(), vals));
    }
    if out.is_empty() {
        return Err(SketchError::BadParam("config spec was empty".into()));
    }
    Ok(out)
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
        fn canonical() -> Self {
            FakeParams {
                rows: 5,
                cols: 2048,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OtherParams {
        lg_k: u8,
    }

    impl SketchParams for OtherParams {
        const FAMILY: &'static str = "other";
        fn canonical() -> Self {
            OtherParams { lg_k: 14 }
        }
    }

    /// A float-valued axis, so the untyped hop is tested against the one
    /// scalar kind `typed` can silently round to the wrong thing.
    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FloatParams {
        alpha: f64,
    }

    impl SketchParams for FloatParams {
        const FAMILY: &'static str = "float";
        fn canonical() -> Self {
            FloatParams { alpha: 0.01 }
        }
    }

    #[test]
    fn single_parses_one_point() {
        let p = ParamSet::single("fake", "rows=5 cols=2048").unwrap();
        assert_eq!(
            p.parse::<FakeParams>().unwrap(),
            FakeParams {
                rows: 5,
                cols: 2048
            }
        );
    }

    #[test]
    fn single_rejects_a_multi_value_axis() {
        // A comma list is a series, which is the caller's job, not this
        // parser's: one invocation, one cell.
        let err = ParamSet::single("fake", "rows=3,5 cols=1024")
            .unwrap_err()
            .to_string();
        assert!(err.contains("rows"), "{err}");
    }

    #[test]
    fn single_defers_key_checking_to_parse() {
        // `single` knows no family, so a misspelled key survives and is
        // caught by serde against the struct that defines the fields.
        let p = ParamSet::single("fake", "rows=5 colz=1024").unwrap();
        let err = p.parse::<FakeParams>().unwrap_err().to_string();
        assert!(err.contains("colz"), "{err}");
    }

    #[test]
    fn single_defers_type_checking_to_parse() {
        let p = ParamSet::single("other", "lg_k=huge").unwrap();
        assert!(p.parse::<OtherParams>().is_err());
    }

    #[test]
    fn single_floats_survive_the_untyped_hop() {
        let p = ParamSet::single("float", "alpha=0.01").unwrap();
        assert!((p.parse::<FloatParams>().unwrap().alpha - 0.01).abs() < 1e-12);
    }

    #[test]
    fn single_rejects_malformed_specs() {
        assert!(ParamSet::single("fake", "rows").is_err());
        assert!(ParamSet::single("fake", "rows=").is_err());
        assert!(ParamSet::single("fake", "").is_err());
    }

    #[test]
    fn empty_is_a_parameterless_point() {
        let p = ParamSet::empty("fake");
        assert_eq!(p.family(), "fake");
        assert_eq!(p.fields(), Vec::<(String, String)>::new());
        // A params struct with required fields rejects it, naming a field.
        assert!(p.parse::<FakeParams>().is_err());
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
