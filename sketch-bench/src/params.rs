//! Construction parameters: the open algorithm axis ([`ParamSet`]) and one
//! vocabulary per sketch family. One struct is one **family** — a variant
//! needing a different knob is a different family.

use aqpbm_datagen::DataGenError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Is `algorithm` the family `family`, or one of its variants?
/// A variant is the family name, a `-`, then its own name — and that `-` is what
/// keeps `countsketch` out of the `cms` family, and `topk-cms` out of it too.
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
    /// The default accepts the family's variants too, since a variant takes the
    /// same knobs. A params type whose name must match exactly overrides this.
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
    pub fn parse<P: SketchParams>(&self) -> Result<P, DataGenError> {
        if !P::owns(&self.algorithm) {
            return Err(DataGenError::BadParam(format!(
                "params are for algorithm '{}', which is not in the '{}' family",
                self.algorithm,
                P::FAMILY
            )));
        }
        serde_json::from_value(self.params.clone())
            .map_err(|e| DataGenError::BadParam(format!("{} params: {e}", self.algorithm)))
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
    /// list is an error, since one invocation measures one point. Syntax only:
    /// no algorithm is named here, so membership is the caller's check.
    pub fn single(algorithm: &str, spec: &str) -> Result<ParamSet, DataGenError> {
        let axes = parse_axes(spec)?;
        let mut params = serde_json::Map::new();
        for (key, values) in axes {
            if values.len() > 1 {
                return Err(DataGenError::BadParam(format!(
                    "config key '{key}' lists {} values; --config takes one value \
                     per key (a single point). Invoke once per point to measure a series.",
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
fn parse_axes(spec: &str) -> Result<Vec<(String, Vec<String>)>, DataGenError> {
    let mut out = Vec::new();
    for tok in spec.split_whitespace() {
        let (key, vals) = tok
            .split_once('=')
            .ok_or_else(|| DataGenError::BadParam(format!("config token missing '=': {tok}")))?;
        let vals: Vec<String> = vals.split(',').map(|s| s.trim().to_string()).collect();
        if vals.is_empty() || vals.iter().any(|v| v.is_empty()) {
            return Err(DataGenError::BadParam(format!(
                "config key '{key}' has an empty value list"
            )));
        }
        out.push((key.trim().to_string(), vals));
    }
    if out.is_empty() {
        return Err(DataGenError::BadParam("config spec was empty".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod param_set_tests {
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
        // parser's: one invocation, one point.
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
}

macro_rules! sketch_params {
    ($ty:ident, $family:literal, $canonical:expr) => {
        impl SketchParams for $ty {
            const FAMILY: &'static str = $family;
            fn canonical() -> Self {
                $canonical
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HllParams {
    pub lg_k: u8,
}
// 14 and not some smaller default: it is the one precision every row in the
// family can build at. `asap_sketchlib` puts the register count in a storage
// type and ships three of them, so its rows exist at 12, 14 and 16 only, and a
// canonical point outside that set would be one no `lib` row could take.
sketch_params!(HllParams, "hll", HllParams { lg_k: 14 });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KllParams {
    pub k: u32,
}
sketch_params!(KllParams, "kll", KllParams { k: 100 });

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DdParams {
    pub alpha: f64,
}
sketch_params!(DdParams, "dd", DdParams { alpha: 0.01 });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmsParams {
    pub rows: usize,
    pub cols: usize,
}
sketch_params!(
    CmsParams,
    "cms",
    CmsParams {
        rows: 3,
        cols: 1024
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmsHeapParams {
    pub rows: usize,
    pub cols: usize,
}
// No `top_k` field: the heap's capacity and the TopK comparator's grading `k`
// are the same compile-time constant (`CMS_HEAP_TOP_K` in
// `wrappers::cms_heap::sketchlib`), so there is nothing here for them to
// silently disagree about. `deny_unknown_fields` turns a `--config` that
// tries to set `top_k` anyway into a named error rather than ignoring it.
sketch_params!(
    CmsHeapParams,
    "cms-heap",
    CmsHeapParams {
        rows: 3,
        cols: 1024
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountSketchParams {
    pub rows: usize,
    pub cols: usize,
}
sketch_params!(
    CountSketchParams,
    "countsketch",
    CountSketchParams {
        rows: 3,
        cols: 1024
    }
);

// ---------- Hydra, one algorithm per cell type ----------
// The cell type is on the algorithm axis because each cell answers a different
// statistic under a different comparator, and each keeps `deny_unknown_fields`.

/// Hydra over Count-Min cells: `rows` / `cols` size the outer grid a
/// subpopulation key hashes into, `cell_rows` / `cell_cols` the counter array
/// inside each cell. Memory is their product, so the two pairs are not alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydraCmsParams {
    pub rows: usize,
    pub cols: usize,
    pub cell_rows: usize,
    pub cell_cols: usize,
}
sketch_params!(
    HydraCmsParams,
    "hydra-cms",
    HydraCmsParams {
        rows: 3,
        cols: 128,
        cell_rows: 3,
        cell_cols: 512
    }
);

/// Hydra over Count Sketch cells: `rows` / `cols` size the outer grid a
/// subpopulation key hashes into, `cell_rows` / `cell_cols` the counter array
/// inside each cell. Memory is their product, so the two pairs are not alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydraCsParams {
    pub rows: usize,
    pub cols: usize,
    pub cell_rows: usize,
    pub cell_cols: usize,
}
sketch_params!(
    HydraCsParams,
    "hydra-cs",
    HydraCsParams {
        rows: 3,
        cols: 128,
        cell_rows: 3,
        cell_cols: 512
    }
);

/// Hydra over HyperLogLog cells: the grid shape and nothing else. The library
/// fixes the cell at `HyperLogLogP14` — 2^14 one-byte registers — so a `lg_k`
/// here would be a knob the row reads and cannot act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydraHllParams {
    pub rows: usize,
    pub cols: usize,
}
sketch_params!(
    HydraHllParams,
    "hydra-hll",
    HydraHllParams { rows: 3, cols: 128 }
);

/// Hydra over KLL cells: the grid shape plus the cell's accuracy parameter.
///
/// `cell_k` is the KLL `k`, named with the `cell_` prefix the Count-Min row
/// uses for the same reason: it sizes the structure inside a cell, and reading
/// it as a grid dimension would understate the footprint by the grid area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydraKllParams {
    pub rows: usize,
    pub cols: usize,
    pub cell_k: u32,
}
sketch_params!(
    HydraKllParams,
    "hydra-kll",
    HydraKllParams {
        rows: 3,
        cols: 128,
        cell_k: 200
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnivMonParams {
    pub heap_size: usize,
    pub sketch_row: usize,
    pub sketch_col: usize,
    pub layer_size: usize,
}
sketch_params!(
    UnivMonParams,
    "univmon",
    UnivMonParams {
        heap_size: 1000,
        sketch_row: 5,
        sketch_col: 2048,
        layer_size: 8
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_to_the_documented_bytes() {
        // Pinned exactly, so a representation change cannot silently alter the
        // record shape. Not a copy of core's same-named test: that one pins a
        // synthetic type, these are the bytes a real algorithm writes.
        let p = ParamSet::of(&CmsParams {
            rows: 5,
            cols: 2048,
        });
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"algorithm":"cms","params":{"cols":2048,"rows":5}}"#
        );
    }

    #[test]
    fn records_written_before_the_open_representation_still_parse() {
        // Declaration order, as the closed enum emitted it.
        for (json, algorithm) in [
            (
                r#"{"algorithm":"cms","params":{"rows":5,"cols":2048}}"#,
                "cms",
            ),
            (r#"{"algorithm":"hll","params":{"lg_k":14}}"#, "hll"),
            (
                r#"{"algorithm":"countsketch","params":{"rows":3,"cols":4096}}"#,
                "countsketch",
            ),
            (
                r#"{"algorithm":"univmon","params":{"layers":8,"max_stream":256}}"#,
                "univmon",
            ),
        ] {
            let p: ParamSet = serde_json::from_str(json).unwrap();
            assert_eq!(p.algorithm(), algorithm);
        }
        let cms: ParamSet =
            serde_json::from_str(r#"{"algorithm":"cms","params":{"rows":5,"cols":2048}}"#).unwrap();
        assert_eq!(
            cms.parse::<CmsParams>().unwrap(),
            CmsParams {
                rows: 5,
                cols: 2048
            }
        );
    }

    #[test]
    fn every_algorithm_ships_a_canonical_config_that_roundtrips() {
        // The canonical config is one buildable point per algorithm — the
        // item-type acceptance tests take it as a valid config per impl. It must
        // erase to a `ParamSet` of its own algorithm and parse back unchanged.
        fn check<P: SketchParams + PartialEq + std::fmt::Debug>() {
            let p = P::canonical();
            let set = ParamSet::of(&p);
            assert_eq!(set.algorithm(), P::FAMILY);
            assert_eq!(set.parse::<P>().unwrap(), p);
        }
        check::<HllParams>();
        check::<KllParams>();
        check::<DdParams>();
        check::<CmsParams>();
        check::<CountSketchParams>();
        check::<HydraCmsParams>();
        check::<HydraCsParams>();
        check::<HydraHllParams>();
        check::<HydraKllParams>();
        check::<UnivMonParams>();
    }

    /// Which algorithm names each vocabulary answers to. Pinned per family
    /// rather than trusted to the prefix rule, because the failure it guards
    /// against is silent: a params type that accidentally owned a neighbouring
    /// family's names would accept that family's config and build at it.
    #[test]
    fn each_family_owns_its_variants_and_no_neighbours() {
        assert!(CmsParams::owns("cms"));
        assert!(CmsParams::owns("cms-fastpath-vector2d"));
        assert!(CmsParams::owns("cms-regularpath-vector2d"));
        assert!(!CmsParams::owns("countsketch"));
        assert!(!CmsParams::owns("topk-cms"));
        assert!(!CmsParams::owns("hydra-cms"));

        assert!(CountSketchParams::owns("countsketch-fastpath-vector2d"));
        assert!(!CountSketchParams::owns("cms"));

        assert!(HllParams::owns("hll"));
        assert!(HllParams::owns("hll-hip"));
        assert!(!HllParams::owns("hydra-hll"));

        assert!(KllParams::owns("kll-percall"));
        assert!(KllParams::owns("kll-cdf"));
        assert!(!KllParams::owns("hydra-kll"));

        assert!(DdParams::owns("dd"));
        assert!(!DdParams::owns("kll"));

        // The three Hydra cell types take different knobs, so they are three
        // families and none of them owns another's name.
        assert!(HydraCmsParams::owns("hydra-cms"));
        assert!(!HydraCmsParams::owns("hydra-hll"));
        assert!(!HydraCmsParams::owns("hydra-cs"));
        assert!(HydraCsParams::owns("hydra-cs"));
        assert!(!HydraHllParams::owns("hydra-kll"));

        assert!(UnivMonParams::owns("univmon-cardinality"));
        assert!(UnivMonParams::owns("univmon-l1-norm"));
        assert!(UnivMonParams::owns("univmon-l2-norm"));
        assert!(UnivMonParams::owns("univmon-entropy"));
        assert!(!UnivMonParams::owns("cms"));
    }
}
