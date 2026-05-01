//! `ParamSet` — typed sketch-construction parameters, one variant
//! per family. Flows through the CLI into wrappers so a single
//! `bench` invocation can sweep a grid of `(family, impl, params)`
//! triples.
//!
//! See `docs/BENCH_SWEEP.md` §4.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HllParams {
    pub lg_k: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KllParams {
    pub k: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmsParams {
    pub rows: usize,
    pub cols: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountSketchParams {
    pub rows: usize,
    pub cols: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElasticParams {
    pub buckets: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NitroParams {
    pub rate: f64,
}

/// DDSketch's single tuning knob — the relative-error guarantee
/// `alpha ∈ (0, 1)`. Smaller `alpha` ⇒ more buckets ⇒ tighter
/// per-quantile error at the cost of memory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DdParams {
    pub alpha: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnivMonParams {
    pub layers: usize,
    pub max_stream: u64,
}

/// Family-tagged sketch parameters.
///
/// `#[serde(tag = "family", content = "params")]` produces
/// `{"family": "hll", "params": {"lg_k": 14}}` — readable in the
/// JSONL stream and survivable for a viewer that doesn't know
/// every variant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", content = "params", rename_all = "snake_case")]
pub enum ParamSet {
    Hll(HllParams),
    Kll(KllParams),
    Cms(CmsParams),
    Countsketch(CountSketchParams),
    Elastic(ElasticParams),
    Nitro(NitroParams),
    Univmon(UnivMonParams),
    Dd(DdParams),
}

impl ParamSet {
    /// Family name — matches the `family` field in
    /// `dispatch::ImplEntry` and the CLI's `--sketch` flag.
    pub fn family(&self) -> &'static str {
        match self {
            ParamSet::Hll(_) => "hll",
            ParamSet::Kll(_) => "kll",
            ParamSet::Cms(_) => "cms",
            ParamSet::Countsketch(_) => "countsketch",
            ParamSet::Elastic(_) => "elastic",
            ParamSet::Nitro(_) => "nitro",
            ParamSet::Univmon(_) => "univmon",
            ParamSet::Dd(_) => "dd",
        }
    }

    /// Serialise into a `serde_json::Value` for embedding in the
    /// v1 record's `sketch_config` field.
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ParamSet -> JSON should not fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hll_roundtrip_json() {
        let p = ParamSet::Hll(HllParams { lg_k: 14 });
        let s = serde_json::to_string(&p).unwrap();
        let back: ParamSet = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn family_tags_are_stable() {
        assert_eq!(ParamSet::Hll(HllParams { lg_k: 10 }).family(), "hll");
        assert_eq!(
            ParamSet::Cms(CmsParams {
                rows: 5,
                cols: 2048
            })
            .family(),
            "cms"
        );
    }

    #[test]
    fn cms_json_shape() {
        let p = ParamSet::Cms(CmsParams {
            rows: 5,
            cols: 2048,
        });
        let v = p.to_json_value();
        assert_eq!(v["family"], "cms");
        assert_eq!(v["params"]["rows"], 5);
        assert_eq!(v["params"]["cols"], 2048);
    }
}
