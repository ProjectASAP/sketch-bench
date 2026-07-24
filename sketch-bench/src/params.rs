//! Construction parameters for each sketch family.
//!
//! These live next to the wrappers that consume them, not in
//! `aqpbm-core`. `aqpbm-core` owns the *open* family axis —
//! the `SketchParams` trait and the type-erased `ParamSet` — and
//! deliberately knows no family names. Concrete families are
//! declared by whoever ships the implementations, which is this
//! crate.
//!
//! See `aqpbm_core::config` for why the axis is open at all.

use serde::{Deserialize, Serialize};

// Re-exported so callers reach the whole parameter vocabulary —
// the open axis from `aqpbm-core` plus this crate's families —
// through one module.
pub use aqpbm_core::config::{ParamSet, SketchParams};

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
sketch_params!(HllParams, "hll", HllParams { lg_k: 10 });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KllParams {
    pub k: u32,
}
sketch_params!(KllParams, "kll", KllParams { k: 100 });

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElasticParams {
    pub buckets: usize,
    pub depth: usize,
}
sketch_params!(
    ElasticParams,
    "elastic",
    ElasticParams {
        buckets: 512,
        depth: 2
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NitroParams {
    pub rate: f64,
}
sketch_params!(NitroParams, "nitro", NitroParams { rate: 0.01 });

/// DDSketch's single tuning knob — the relative-error guarantee
/// `alpha ∈ (0, 1)`. Smaller `alpha` ⇒ more buckets ⇒ tighter
/// per-quantile error at the cost of memory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DdParams {
    pub alpha: f64,
}
sketch_params!(DdParams, "dd", DdParams { alpha: 0.005 });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnivMonParams {
    pub layers: usize,
    pub max_stream: u64,
}
sketch_params!(
    UnivMonParams,
    "univmon",
    UnivMonParams {
        layers: 6,
        max_stream: 128
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_to_the_documented_bytes() {
        // Pinned exactly, so a future representation change cannot silently
        // alter the record shape the way the enum -> Value move altered key
        // order. Asserting only field *values* would not have caught that.
        let p = ParamSet::of(&CmsParams {
            rows: 5,
            cols: 2048,
        });
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"family":"cms","params":{"cols":2048,"rows":5}}"#
        );
    }

    #[test]
    fn records_written_before_the_open_representation_still_parse() {
        // Declaration order, as the closed enum emitted it.
        for (json, family) in [
            (r#"{"family":"cms","params":{"rows":5,"cols":2048}}"#, "cms"),
            (r#"{"family":"hll","params":{"lg_k":14}}"#, "hll"),
            (
                r#"{"family":"countsketch","params":{"rows":3,"cols":4096}}"#,
                "countsketch",
            ),
            (
                r#"{"family":"univmon","params":{"layers":8,"max_stream":256}}"#,
                "univmon",
            ),
        ] {
            let p: ParamSet = serde_json::from_str(json).unwrap();
            assert_eq!(p.family(), family);
        }
        let cms: ParamSet =
            serde_json::from_str(r#"{"family":"cms","params":{"rows":5,"cols":2048}}"#).unwrap();
        assert_eq!(
            cms.parse::<CmsParams>().unwrap(),
            CmsParams {
                rows: 5,
                cols: 2048
            }
        );
    }

    #[test]
    fn roundtrips_through_json() {
        let p = ParamSet::of(&HllParams { lg_k: 14 });
        let back: ParamSet = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.parse::<HllParams>().unwrap().lg_k, 14);
    }

    #[test]
    fn parsing_as_the_wrong_family_fails() {
        let p = ParamSet::of(&HllParams { lg_k: 14 });
        let err = p.parse::<CmsParams>().unwrap_err().to_string();
        assert!(err.contains("hll") && err.contains("cms"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected_by_name() {
        // What the hand-written per-family allowed-key lists used to do.
        let p = ParamSet {
            family: "cms".into(),
            params: serde_json::json!({"rows": 5, "colz": 2048}),
        };
        let err = p.parse::<CmsParams>().unwrap_err().to_string();
        assert!(err.contains("colz"), "error should name the bad key: {err}");
    }

    #[test]
    fn every_family_ships_a_canonical_config_that_roundtrips() {
        // The canonical config is one buildable point per family — the
        // dtype-acceptance tests take it as a valid config per impl. It must
        // erase to a `ParamSet` of its own family and parse back unchanged.
        fn check<P: SketchParams + PartialEq + std::fmt::Debug>() {
            let p = P::canonical();
            let set = ParamSet::of(&p);
            assert_eq!(set.family(), P::FAMILY);
            assert_eq!(set.parse::<P>().unwrap(), p);
        }
        check::<HllParams>();
        check::<KllParams>();
        check::<CmsParams>();
        check::<CountSketchParams>();
        check::<ElasticParams>();
        check::<UnivMonParams>();
        check::<DdParams>();
        check::<NitroParams>();
    }

    #[test]
    fn fields_are_ordered_and_stringified() {
        let p = ParamSet::of(&CmsParams {
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
