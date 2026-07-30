//! Accumulator-construction parameters.
//!
//! The algorithm axis is open: [`ParamSet`] carries the algorithm name plus its
//! parameters as JSON, and each params type declares its own name, its own
//! canonical config, and — through serde — its own parsing and field names.
//!
//! One params type is one **family**. An algorithm named `cms` and one named
//! `cms-fastpath-vector2d` build from the same `{rows, cols}` vocabulary, so
//! they are one family and one params type serves both. Which variants exist is
//! the catalog's business; which vocabulary they share is this type's.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use aqpbm_datagen::SketchError;

/// Is `algorithm` the family `family`, or one of its variants?
///
/// A variant is the family name, a `-`, then the variant's own name. The rule
/// lives here so the one place an algorithm name is related to its parameter
/// vocabulary is one function, not a prefix test repeated per params type.
///
/// The `-` is what keeps `countsketch` out of the `cms` family, and
/// `topk-cms` out of it too.
pub fn in_family(algorithm: &str, family: &str) -> bool {
    algorithm == family
        || algorithm
            .strip_prefix(family)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Construction parameters for one sketch family. `deny_unknown_fields` on each
/// implementor turns a typo in `--config` into an error naming the offending key.
pub trait SketchParams: Serialize + DeserializeOwned + Clone + std::fmt::Debug {
    /// The family this vocabulary names. Also the algorithm name of the family's
    /// base row, the one declaring no variant.
    const FAMILY: &'static str;

    /// Does the algorithm named `algorithm` build from this vocabulary?
    ///
    /// The default accepts the family's variants as well as the family itself,
    /// because a variant is a different way of implementing the same structure
    /// and takes the same knobs. A params type whose name must match exactly
    /// overrides this.
    fn owns(algorithm: &str) -> bool {
        in_family(algorithm, Self::FAMILY)
    }

    /// One representative, buildable config for the family. Lives on the params
    /// type, not a table keyed by algorithm name, so it cannot drift from what it
    /// configures. Not a sweep — it is the single point acceptance tests build.
    fn canonical() -> Self;
}

/// Algorithm-tagged parameters, type-erased so the set of algorithms stays open.
/// Serialises as `{"algorithm": "...", "params": {...}}`, the shape of a record's
/// `sketch_config` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSet {
    pub algorithm: String,
    pub params: serde_json::Value,
}

impl ParamSet {
    /// Erase a typed params value, tagged with the family's base algorithm.
    /// [`Self::of_algorithm`] is the version that names a variant.
    pub fn of<P: SketchParams>(p: &P) -> Self {
        Self::of_algorithm(P::FAMILY, p)
    }

    /// Erase a typed params value under a named algorithm, which must be one
    /// `P` owns. Panics otherwise: a caller naming an algorithm from another
    /// family has a bug this cannot paper over.
    pub fn of_algorithm<P: SketchParams>(algorithm: &str, p: &P) -> Self {
        assert!(
            P::owns(algorithm),
            "algorithm '{algorithm}' does not build from the '{}' vocabulary",
            P::FAMILY
        );
        Self {
            algorithm: algorithm.to_string(),
            params: serde_json::to_value(p).expect("params -> JSON should not fail"),
        }
    }

    /// Recover the typed value. Fails if this set belongs to another family, or
    /// if the JSON does not match `P` — which is how a misspelled `--config` key
    /// is reported, with serde naming it and listing the valid ones.
    pub fn parse<P: SketchParams>(&self) -> Result<P, SketchError> {
        if !P::owns(&self.algorithm) {
            return Err(SketchError::BadParam(format!(
                "params are for algorithm '{}', which is not in the '{}' family",
                self.algorithm,
                P::FAMILY
            )));
        }
        serde_json::from_value(self.params.clone())
            .map_err(|e| SketchError::BadParam(format!("{} params: {e}", self.algorithm)))
    }

    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// The whole tagged object, for the record's `sketch_config` field.
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ParamSet -> JSON should not fail")
    }

    /// The parameterless point for an algorithm — what `--config` defaults to. An
    /// impl needing parameters rejects it, naming the field it is missing.
    pub fn empty(algorithm: &str) -> Self {
        Self {
            algorithm: algorithm.to_string(),
            params: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Parse one `--config` point: `'k1=v1 k2=v2'`, one value per key — a comma
    /// list is an error, since one invocation measures one cell. Syntax only:
    /// no algorithm is named here, so membership is the caller's check.
    pub fn single(algorithm: &str, spec: &str) -> Result<ParamSet, SketchError> {
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
            algorithm: algorithm.to_string(),
            params: serde_json::Value::Object(params),
        })
    }

    /// `(key, value)` pairs of the parameters, ordered by key — lets the CSV
    /// writer emit a header and a row for any algorithm with no per-algorithm table,
    /// and keeps one file's header stable across runs.
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

    /// Algorithms declared right here, so these tests exercise the open axis
    /// without `aqpbm-core` knowing any real algorithm. That the axis can be
    /// exercised this way *is* the property under test.
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

    /// The rule that decides whether an algorithm builds from a given
    /// vocabulary. The `-` matters: without it `countsketch` would answer to
    /// the `cms` vocabulary and a `cms` config would be accepted by a
    /// CountSketch row.
    #[test]
    fn a_family_covers_its_variants_and_nothing_else() {
        assert!(in_family("cms", "cms"));
        assert!(in_family("cms-fastpath-vector2d", "cms"));
        assert!(!in_family("countsketch", "cms"));
        assert!(!in_family("topk-cms", "cms"));
        assert!(!in_family("cmsx", "cms"));
        assert!(!in_family("cm", "cms"));
    }

    /// A variant keeps its own name in the record while parsing through the
    /// family's vocabulary — the property that lets one params type serve every
    /// variant without the record losing which one ran.
    #[test]
    fn a_variant_parses_through_its_family_vocabulary() {
        let p = ParamSet::of_algorithm("fake-fastpath", &FakeParams::canonical());
        assert_eq!(p.algorithm(), "fake-fastpath");
        assert_eq!(p.parse::<FakeParams>().unwrap(), FakeParams::canonical());
    }

    #[test]
    #[should_panic(expected = "does not build from")]
    fn naming_an_algorithm_from_another_family_panics() {
        ParamSet::of_algorithm("other-hip", &FakeParams::canonical());
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
        // `single` knows no algorithm, so a misspelled key survives and is
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
        assert_eq!(p.algorithm(), "fake");
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
            r#"{"algorithm":"fake","params":{"cols":2048,"rows":5}}"#
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
    fn parsing_as_the_wrong_algorithm_fails() {
        let p = ParamSet::of(&OtherParams { lg_k: 14 });
        let err = p.parse::<FakeParams>().unwrap_err().to_string();
        assert!(err.contains("other") && err.contains("fake"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected_by_name() {
        // `deny_unknown_fields` is what names the offending key.
        let p = ParamSet {
            algorithm: "fake".into(),
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
