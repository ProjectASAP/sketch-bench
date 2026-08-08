//! Construction parameters for each sketch family. These live next to the
//! wrappers that consume them, not in `aqpbm-core`: the core owns the *open*
//! algorithm axis and deliberately knows no algorithm names, so concrete algorithms are
//! declared by whoever ships the implementations. See `aqpbm_core::config`.
//!
//! One struct is one **family**, not one algorithm. Five algorithms measure
//! five Count-Min structures — `cms`, `cms-fastpath-vector2d`,
//! `cms-regularpath-vector2d`, `cms-fastpath-fixedmatrix` and
//! `cms-fastpath-fixedmatrix-32k-parallel` — but a reader configures every one
//! of them with the same `rows` and `cols`, so they share [`CmsParams`] and the
//! family name it declares. A structural variant that needed a *different* knob would be a
//! different family, which is why the three Hydra cell types have three structs.

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

/// Top-k is its own algorithm because its parameter vocabulary is: `k` sizes the
/// candidate tracker, `rows`/`cols` the counter array under it. Sharing
/// `CmsParams` would give every CMS row a `k` that means nothing to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopkParams {
    pub rows: usize,
    pub cols: usize,
    /// How many heaviest keys the tracker keeps. Sizes both the sketch's
    /// candidate set and the prefix the comparator scores against.
    pub k: usize,
}
sketch_params!(
    TopkParams,
    "topk",
    TopkParams {
        rows: 5,
        cols: 2048,
        k: 100
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

// ---------- Hydra, one algorithm per cell type ----------
//
// The cell type is on the algorithm axis, not the impl axis, because each cell
// answers a different statistic and is scored by a different comparator. See
// `docs/sketch-bench.md` on what makes a panel meaningful. All three come from
// `asap_sketchlib::sketch_framework::Hydra`, so all three have one impl, `lib`.
//
// Splitting them is also what lets each keep an ordinary params struct with
// `deny_unknown_fields` on: a single `hydra` algorithm carrying every cell
// type's knobs would have to accept `cell_k` on a Count-Min row and ignore it.

/// Hydra over Count-Min cells: two nested shapes, so two pairs of dimensions.
/// `rows` and `cols` size the outer grid a subpopulation key hashes into,
/// `cell_rows` and `cell_cols` size the counter array inside every one of those
/// cells.
///
/// Memory is their product, so the two pairs are not interchangeable knobs: a
/// grid of 1024 columns holding 2048-column cells is three orders of magnitude
/// past either one alone.
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

/// Hydra over HyperLogLog cells: the grid shape and nothing else.
///
/// The library fixes the cell at `HyperLogLog<ErtlMLE>`, which is
/// `HyperLogLogP14`, so a cell is 2^14 one-byte registers and there is no cell
/// parameter to expose. A `lg_k` here would be a knob the row reads and cannot
/// act on, which is the defect #58 records against the fixed-shape rows.
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
        //
        // `aqpbm_core::config` has a test of this name too, and this one is
        // not a copy of it. That one pins the bytes of a synthetic params
        // type, which nothing outside the test reads. These are the bytes a
        // real algorithm writes into `sketch_config`, which is a contract with
        // whoever reads the records. The generic behaviour the two share is
        // core's to test, and this file tests it once, here, where the bytes
        // mean something.
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
            (r#"{"algorithm":"cms","params":{"rows":5,"cols":2048}}"#, "cms"),
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
        check::<ElasticParams>();
        check::<UnivMonParams>();
        check::<DdParams>();
        check::<NitroParams>();
        check::<TopkParams>();
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

        assert!(TopkParams::owns("topk-cms"));
        assert!(!TopkParams::owns("cms"));

        // The three Hydra cell types take different knobs, so they are three
        // families and none of them owns another's name.
        assert!(HydraCmsParams::owns("hydra-cms"));
        assert!(!HydraCmsParams::owns("hydra-hll"));
        assert!(!HydraHllParams::owns("hydra-kll"));
    }
}
