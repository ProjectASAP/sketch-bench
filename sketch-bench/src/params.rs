//! Construction parameters for each sketch family, next to the wrappers that
//! consume them because `aqpbm-core` owns the *open* algorithm axis. One struct
//! is one **family**: a variant needing a different knob is a different family.

use serde::{Deserialize, Serialize};

// Re-exported so callers reach the whole parameter vocabulary —
// the open axis from `aqpbm-core` plus this crate's families —
// through one module.
pub use aqpbm_core::config::{in_family, ParamSet, SketchParams};

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
        check::<CmsParams>();
        check::<CountSketchParams>();
        check::<HydraCmsParams>();
        check::<HydraHllParams>();
        check::<HydraKllParams>();
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

        // The three Hydra cell types take different knobs, so they are three
        // families and none of them owns another's name.
        assert!(HydraCmsParams::owns("hydra-cms"));
        assert!(!HydraCmsParams::owns("hydra-hll"));
        assert!(!HydraHllParams::owns("hydra-kll"));
    }
}
