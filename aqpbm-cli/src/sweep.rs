//! `--config 'k=v1,v2 k2=v3,v4'` → a Cartesian product of [`ParamSet`]s.
//!
//! This was eight `build_<family>` functions, each with a hand-written list of
//! allowed keys, a hand-written arity check and a hand-written cross product —
//! 51 branches that had to be extended for every new family, and whose failure
//! mode was a `--config` key silently belonging to no family.
//!
//! None of that was family-specific work. Splitting `k=v1,v2` into a grid is
//! the same operation whatever the keys mean; deciding whether a key exists
//! and whether its value has the right type is what serde does, driven by the
//! params struct itself. So the parser builds an untyped JSON object per grid
//! point and hands it to [`ParamSet::parse`], performed by the caller against
//! the concrete type. `deny_unknown_fields` turns a typo into an error naming
//! the key and listing the valid ones — better than the old list, and nobody
//! has to maintain it.
//!
//! See `docs/BENCH_SWEEP.md` §2 + §3.

use crate::params::ParamSet;
use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

/// Default grid for a family, read from the dispatch table — which is where
/// the family's params type is already named — rather than from a second
/// table keyed by family string that could disagree with it.
pub fn default_grid(family: &str) -> Result<Vec<ParamSet>> {
    let entry = crate::dispatch::impls_for_family(family)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("unknown sketch family: {family}"))?;
    Ok((entry.params.default_grid)())
}

/// Expand `--config` into one `ParamSet` per grid point.
///
/// Values are typed by content: `5` becomes a JSON number, `1.1` a float,
/// `true` a bool, anything else a string. That is what lets one untyped
/// parser feed eight differently-shaped params structs — serde does the final
/// coercion and rejects mismatches by field name.
pub fn parse_config(family: &str, spec: &str) -> Result<Vec<ParamSet>> {
    // The old per-family dispatch rejected an unknown family here. The CLI
    // also bails earlier, in `select_impls`, but this is `pub` and the
    // guarantee should live in the function rather than in the order its
    // callers happen to run.
    if crate::dispatch::impls_for_family(family).is_empty() {
        bail!("unknown sketch family: {family}");
    }
    let kvs = parse_kvs(spec)?;
    let mut grid: Vec<Map<String, Value>> = vec![Map::new()];
    for (key, values) in &kvs {
        let mut next = Vec::with_capacity(grid.len() * values.len());
        for base in &grid {
            for v in values {
                let mut row = base.clone();
                row.insert(key.clone(), typed(v));
                next.push(row);
            }
        }
        grid = next;
    }
    Ok(grid
        .into_iter()
        .map(|params| ParamSet {
            family: family.to_string(),
            params: Value::Object(params),
        })
        .collect())
}

/// `"5"` → number, `"1.1"` → float, `"true"` → bool, else string.
fn typed(raw: &str) -> Value {
    if let Ok(i) = raw.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Value::from(f);
    }
    if let Ok(b) = raw.parse::<bool>() {
        return Value::from(b);
    }
    Value::from(raw)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{CmsParams, DdParams, HllParams};

    #[test]
    fn expands_a_cartesian_product() {
        let grid = parse_config("cms", "rows=3,5 cols=1024,2048").unwrap();
        assert_eq!(grid.len(), 4);
        let typed: Vec<CmsParams> = grid.iter().map(|p| p.parse().unwrap()).collect();
        assert!(typed.contains(&CmsParams {
            rows: 3,
            cols: 1024
        }));
        assert!(typed.contains(&CmsParams {
            rows: 5,
            cols: 2048
        }));
    }

    #[test]
    fn single_key_single_value() {
        let grid = parse_config("hll", "lg_k=14").unwrap();
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].parse::<HllParams>().unwrap().lg_k, 14);
    }

    #[test]
    fn a_misspelled_key_is_rejected_by_name() {
        // The job the per-family allowed-key lists used to do, now done by
        // serde against the struct that defines the fields.
        let grid = parse_config("cms", "rows=5 colz=1024").unwrap();
        let err = grid[0].parse::<CmsParams>().unwrap_err().to_string();
        assert!(err.contains("colz"), "{err}");
    }

    #[test]
    fn a_wrongly_typed_value_is_rejected() {
        let grid = parse_config("hll", "lg_k=huge").unwrap();
        assert!(grid[0].parse::<HllParams>().is_err());
    }

    #[test]
    fn floats_survive_the_untyped_hop() {
        let grid = parse_config("dd", "alpha=0.01,0.05").unwrap();
        let typed: Vec<DdParams> = grid.iter().map(|p| p.parse().unwrap()).collect();
        assert_eq!(typed.len(), 2);
        assert!((typed[0].alpha - 0.01).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_family_is_rejected() {
        assert!(parse_config("not_a_family", "k=1").is_err());
    }

    #[test]
    fn malformed_specs_are_errors() {
        assert!(parse_config("cms", "rows").is_err());
        assert!(parse_config("cms", "rows=").is_err());
        assert!(parse_config("cms", "").is_err());
    }
}
