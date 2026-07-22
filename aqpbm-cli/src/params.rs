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
    ($ty:ident, $family:literal, $grid:expr) => {
        impl SketchParams for $ty {
            const FAMILY: &'static str = $family;
            fn default_grid() -> Vec<Self> {
                $grid
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HllParams {
    pub lg_k: u8,
}
sketch_params!(
    HllParams,
    "hll",
    [10u8, 12, 14, 16]
        .iter()
        .map(|&lg_k| HllParams { lg_k })
        .collect()
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KllParams {
    pub k: u32,
}
sketch_params!(
    KllParams,
    "kll",
    [100u32, 200, 400, 800]
        .iter()
        .map(|&k| KllParams { k })
        .collect()
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmsParams {
    pub rows: usize,
    pub cols: usize,
}
sketch_params!(
    CmsParams,
    "cms",
    grid2(&[3, 5, 7], &[1024, 2048, 4096], |rows, cols| CmsParams {
        rows,
        cols
    })
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
    grid2(&[3, 5, 7], &[1024, 2048, 4096], |rows, cols| {
        CountSketchParams { rows, cols }
    })
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
    grid2(&[512, 1024, 2048], &[2, 3, 4], |buckets, depth| {
        ElasticParams { buckets, depth }
    })
);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NitroParams {
    pub rate: f64,
}
sketch_params!(
    NitroParams,
    "nitro",
    [0.01f64, 0.02, 0.05, 0.10]
        .iter()
        .map(|&rate| NitroParams { rate })
        .collect()
);

/// DDSketch's single tuning knob — the relative-error guarantee
/// `alpha ∈ (0, 1)`. Smaller `alpha` ⇒ more buckets ⇒ tighter
/// per-quantile error at the cost of memory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DdParams {
    pub alpha: f64,
}
sketch_params!(
    DdParams,
    "dd",
    [0.005f64, 0.01, 0.02, 0.05, 0.1]
        .iter()
        .map(|&alpha| DdParams { alpha })
        .collect()
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnivMonParams {
    pub layers: usize,
    pub max_stream: u64,
}
sketch_params!(
    UnivMonParams,
    "univmon",
    grid2(
        &[6usize, 8, 10],
        &[128u64, 256, 512],
        |layers, max_stream| { UnivMonParams { layers, max_stream } }
    )
);

/// Cartesian product of two axes — the shape most default grids have.
fn grid2<A: Copy, B: Copy, T>(a: &[A], b: &[B], f: impl Fn(A, B) -> T) -> Vec<T> {
    a.iter()
        .flat_map(|&x| b.iter().map(move |&y| (x, y)))
        .map(|(x, y)| f(x, y))
        .collect()
}

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
    fn every_family_ships_a_non_empty_default_grid() {
        assert!(!HllParams::default_grid().is_empty());
        assert!(!KllParams::default_grid().is_empty());
        assert_eq!(CmsParams::default_grid().len(), 9);
        assert_eq!(CountSketchParams::default_grid().len(), 9);
        assert_eq!(ElasticParams::default_grid().len(), 9);
        assert_eq!(UnivMonParams::default_grid().len(), 9);
        assert_eq!(DdParams::default_grid().len(), 5);
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
