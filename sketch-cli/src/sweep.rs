//! Config sweep support: per-family default grids and a parser
//! for `--config 'k=v1,v2 k2=v3,v4'` strings that expands into a
//! `Vec<ParamSet>` via Cartesian product.
//!
//! See `docs/BENCH_SWEEP.md` §2 + §3.

use anyhow::{anyhow, bail, Result};
use sketch_core::config::{
    CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, KllParams, NitroParams,
    ParamSet, UnivMonParams,
};

/// Default grid for a family — used when `--config` is omitted.
/// Numbers live here rather than in `docs/BENCH_SWEEP.md` so a
/// `git blame` on the runtime value always matches the code.
pub fn default_grid(family: &str) -> Result<Vec<ParamSet>> {
    Ok(match family {
        "hll" => [10u8, 12, 14, 16]
            .iter()
            .map(|&lg_k| ParamSet::Hll(HllParams { lg_k }))
            .collect(),
        "kll" => [100u32, 200, 400, 800]
            .iter()
            .map(|&k| ParamSet::Kll(KllParams { k }))
            .collect(),
        "cms" => {
            let mut out = Vec::new();
            for &rows in &[3usize, 5, 7] {
                for &cols in &[1024usize, 2048, 4096] {
                    out.push(ParamSet::Cms(CmsParams { rows, cols }));
                }
            }
            out
        }
        "countsketch" => {
            let mut out = Vec::new();
            for &rows in &[3usize, 5, 7] {
                for &cols in &[1024usize, 2048, 4096] {
                    out.push(ParamSet::Countsketch(CountSketchParams { rows, cols }));
                }
            }
            out
        }
        "elastic" => {
            let mut out = Vec::new();
            for &buckets in &[512usize, 1024, 2048] {
                for &depth in &[2usize, 3, 4] {
                    out.push(ParamSet::Elastic(ElasticParams { buckets, depth }));
                }
            }
            out
        }
        "nitro" => [0.01f64, 0.02, 0.05, 0.10]
            .iter()
            .map(|&rate| ParamSet::Nitro(NitroParams { rate }))
            .collect(),
        "univmon" => {
            let mut out = Vec::new();
            for &layers in &[6usize, 8, 10] {
                for &max_stream in &[128u64, 256, 512] {
                    out.push(ParamSet::Univmon(UnivMonParams { layers, max_stream }));
                }
            }
            out
        }
        "dd" => [0.005f64, 0.01, 0.02, 0.05, 0.1]
            .iter()
            .map(|&alpha| ParamSet::Dd(DdParams { alpha }))
            .collect(),
        other => bail!("no default grid for sketch family: {other}"),
    })
}

/// Parse a `--config 'k=v1,v2 k2=v3,v4'` string and expand to a
/// Cartesian product. Whitespace separates keys; commas separate
/// values within a key.
///
/// Example: `"rows=3,5 cols=1024,2048"` → 4 `CmsParams`.
pub fn parse_config(family: &str, spec: &str) -> Result<Vec<ParamSet>> {
    let kvs = parse_kvs(spec)?;
    match family {
        "hll" => build_hll(&kvs),
        "kll" => build_kll(&kvs),
        "cms" => build_cms(&kvs),
        "countsketch" => build_countsketch(&kvs),
        "elastic" => build_elastic(&kvs),
        "nitro" => build_nitro(&kvs),
        "univmon" => build_univmon(&kvs),
        "dd" => build_dd(&kvs),
        other => bail!("unknown sketch family: {other}"),
    }
}

fn parse_kvs(spec: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut out = Vec::new();
    for tok in spec.split_whitespace() {
        let (key, vals) = tok
            .split_once('=')
            .ok_or_else(|| anyhow!("config token missing '=': {tok}"))?;
        let vals: Vec<String> = vals.split(',').map(|s| s.trim().to_string()).collect();
        if vals.is_empty() || vals.iter().any(|v| v.is_empty()) {
            bail!("config key '{key}' has an empty value list");
        }
        out.push((key.trim().to_string(), vals));
    }
    if out.is_empty() {
        bail!("--config was empty");
    }
    Ok(out)
}

fn take_single_key<'a>(
    kvs: &'a [(String, Vec<String>)],
    family: &str,
    allowed: &[&str],
) -> Result<&'a [String]> {
    check_allowed_keys(kvs, family, allowed)?;
    if kvs.len() != 1 {
        bail!("{family} takes exactly one key, got {}", kvs.len());
    }
    Ok(&kvs[0].1)
}

fn check_allowed_keys(kvs: &[(String, Vec<String>)], family: &str, allowed: &[&str]) -> Result<()> {
    for (k, _) in kvs {
        if !allowed.iter().any(|a| a == k) {
            bail!(
                "unknown key '{k}' for {family}; allowed: {}",
                allowed.join(", ")
            );
        }
    }
    Ok(())
}

fn get_values<'a>(kvs: &'a [(String, Vec<String>)], key: &str) -> &'a [String] {
    kvs.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[])
}

// ---------- per-family builders ----------

fn build_hll(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    let vals = take_single_key(kvs, "hll", &["lg_k"])?;
    vals.iter()
        .map(|v| {
            let lg_k: u8 = v.parse().map_err(|_| anyhow!("bad lg_k value: {v}"))?;
            Ok(ParamSet::Hll(HllParams { lg_k }))
        })
        .collect()
}

fn build_kll(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    let vals = take_single_key(kvs, "kll", &["k"])?;
    vals.iter()
        .map(|v| {
            let k: u32 = v.parse().map_err(|_| anyhow!("bad k value: {v}"))?;
            Ok(ParamSet::Kll(KllParams { k }))
        })
        .collect()
}

fn build_cms(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    check_allowed_keys(kvs, "cms", &["rows", "cols"])?;
    let rows = parse_usize_list(get_values(kvs, "rows"), "rows", &[5])?;
    let cols = parse_usize_list(get_values(kvs, "cols"), "cols", &[2048])?;
    let mut out = Vec::new();
    for &r in &rows {
        for &c in &cols {
            out.push(ParamSet::Cms(CmsParams { rows: r, cols: c }));
        }
    }
    Ok(out)
}

fn build_countsketch(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    check_allowed_keys(kvs, "countsketch", &["rows", "cols"])?;
    let rows = parse_usize_list(get_values(kvs, "rows"), "rows", &[5])?;
    let cols = parse_usize_list(get_values(kvs, "cols"), "cols", &[2048])?;
    let mut out = Vec::new();
    for &r in &rows {
        for &c in &cols {
            out.push(ParamSet::Countsketch(CountSketchParams {
                rows: r,
                cols: c,
            }));
        }
    }
    Ok(out)
}

fn build_elastic(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    check_allowed_keys(kvs, "elastic", &["buckets", "depth"])?;
    let buckets = parse_usize_list(get_values(kvs, "buckets"), "buckets", &[1024])?;
    let depths = parse_usize_list(get_values(kvs, "depth"), "depth", &[3])?;
    let mut out = Vec::new();
    for &b in &buckets {
        for &d in &depths {
            out.push(ParamSet::Elastic(ElasticParams {
                buckets: b,
                depth: d,
            }));
        }
    }
    Ok(out)
}

fn build_nitro(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    let vals = take_single_key(kvs, "nitro", &["rate"])?;
    vals.iter()
        .map(|v| {
            let rate: f64 = v.parse().map_err(|_| anyhow!("bad rate value: {v}"))?;
            Ok(ParamSet::Nitro(NitroParams { rate }))
        })
        .collect()
}

fn build_univmon(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    check_allowed_keys(kvs, "univmon", &["layers", "max_stream"])?;
    let layers = parse_usize_list(get_values(kvs, "layers"), "layers", &[8])?;
    let max_stream_strs = get_values(kvs, "max_stream");
    let max_stream = if max_stream_strs.is_empty() {
        vec![256u64]
    } else {
        max_stream_strs
            .iter()
            .map(|v| v.parse::<u64>().map_err(|_| anyhow!("bad max_stream: {v}")))
            .collect::<Result<Vec<_>>>()?
    };
    let mut out = Vec::new();
    for &l in &layers {
        for &m in &max_stream {
            out.push(ParamSet::Univmon(UnivMonParams {
                layers: l,
                max_stream: m,
            }));
        }
    }
    Ok(out)
}

fn build_dd(kvs: &[(String, Vec<String>)]) -> Result<Vec<ParamSet>> {
    let vals = take_single_key(kvs, "dd", &["alpha"])?;
    vals.iter()
        .map(|v| {
            let alpha: f64 = v.parse().map_err(|_| anyhow!("bad alpha value: {v}"))?;
            Ok(ParamSet::Dd(DdParams { alpha }))
        })
        .collect()
}

fn parse_usize_list(vals: &[String], key: &str, fallback: &[usize]) -> Result<Vec<usize>> {
    if vals.is_empty() {
        Ok(fallback.to_vec())
    } else {
        vals.iter()
            .map(|v| v.parse::<usize>().map_err(|_| anyhow!("bad {key}: {v}")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hll_default_grid_has_four_configs() {
        let g = default_grid("hll").unwrap();
        assert_eq!(g.len(), 4);
        assert!(matches!(g[0], ParamSet::Hll(HllParams { lg_k: 10 })));
    }

    #[test]
    fn cms_default_grid_is_cartesian_3x3() {
        let g = default_grid("cms").unwrap();
        assert_eq!(g.len(), 9);
    }

    #[test]
    fn parse_hll_single_value() {
        let g = parse_config("hll", "lg_k=14").unwrap();
        assert_eq!(g, vec![ParamSet::Hll(HllParams { lg_k: 14 })]);
    }

    #[test]
    fn parse_hll_multi_value() {
        let g = parse_config("hll", "lg_k=10,12,14").unwrap();
        assert_eq!(g.len(), 3);
    }

    #[test]
    fn parse_cms_cartesian() {
        let g = parse_config("cms", "rows=3,5 cols=1024,2048").unwrap();
        assert_eq!(g.len(), 4);
        // Order: rows-major (outer), cols (inner).
        if let ParamSet::Cms(p) = &g[0] {
            assert_eq!(p.rows, 3);
            assert_eq!(p.cols, 1024);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_rejects_unknown_key() {
        let err = parse_config("cms", "magic=1").unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[test]
    fn parse_cms_single_key_uses_fallback() {
        // Only rows specified; cols defaults to [2048].
        let g = parse_config("cms", "rows=3,5,7").unwrap();
        assert_eq!(g.len(), 3);
        if let ParamSet::Cms(p) = &g[0] {
            assert_eq!(p.cols, 2048);
        }
    }

    #[test]
    fn parse_unknown_family() {
        assert!(parse_config("whatever", "k=1").is_err());
    }
}
