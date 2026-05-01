//! Default `ParamSet` per family — the single-config fallback
//! when neither `--config` nor a sweep grid overrides it. These
//! values reproduce the knobs the 21 legacy binaries hard-coded.

use sketch_core::config::{
    CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, KllParams, NitroParams,
    ParamSet, UnivMonParams,
};

/// Default params for a family name (as used by `--sketch`).
/// Returns `None` for unknown families.
#[allow(dead_code)]
pub fn default_params(family: &str) -> Option<ParamSet> {
    Some(match family {
        "hll" => ParamSet::Hll(HllParams { lg_k: 14 }),
        "kll" => ParamSet::Kll(KllParams { k: 200 }),
        "cms" => ParamSet::Cms(CmsParams {
            rows: 5,
            cols: 2048,
        }),
        "countsketch" => ParamSet::Countsketch(CountSketchParams {
            rows: 5,
            cols: 2048,
        }),
        "elastic" => ParamSet::Elastic(ElasticParams {
            buckets: 1024,
            depth: 3,
        }),
        "nitro" => ParamSet::Nitro(NitroParams { rate: 0.01 }),
        "univmon" => ParamSet::Univmon(UnivMonParams {
            layers: 8,
            max_stream: 256,
        }),
        "dd" => ParamSet::Dd(DdParams { alpha: 0.01 }),
        _ => return None,
    })
}
