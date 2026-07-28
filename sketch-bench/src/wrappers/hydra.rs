//! Hydra wrapper — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022), the
//! grid-of-sketches that answers per-subpopulation queries out of one shared
//! structure.
//!
//! Two things separate this row from every other one in the catalog.
//!
//! Its item is a **record**, not a key: a stream of `d` label columns plus a
//! value, so it ingests `Labeled<i64>` and reads its workload from a column
//! list. And its insert **fans out**: one record is written into every non-empty
//! subset of its labels, so `d` labels cost `2^d - 1` cell insertions. The
//! reported throughput is records per second, which is the only denominator
//! comparable across `d`; multiply by `2^d - 1` for cell insertions.
//!
//! With a Count-Min counter in each cell, the statistic is
//! [`SubpopFrequencyOps`]: how often a value occurred *within* a subpopulation.
//! The size of the subpopulation itself is a different statistic that a
//! Count-Min cell cannot answer, and would want an HLL-celled row.

use asap_sketchlib::input::HydraCounter;
use asap_sketchlib::{CountMin, DataInput, FastPath, Hydra, Vector2D};

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::accuracy::SubpopFrequencyOps;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;

use crate::params::HydraParams;

/// Hydra over Count-Min cells.
pub struct HydraCms {
    inner: Hydra,
    params: HydraParams,
}

impl InitSketch for HydraCms {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HydraParams = config.parse()?;
        for (name, v) in [
            ("rows", p.rows),
            ("cols", p.cols),
            ("cell_rows", p.cell_rows),
            ("cell_cols", p.cell_cols),
        ] {
            if v == 0 {
                return Err(BuildError(format!("hydra: {name} must be > 0")));
            }
        }
        let cell = HydraCounter::CM(CountMin::<Vector2D<i32>, FastPath>::with_dimensions(
            p.cell_rows,
            p.cell_cols,
        ));
        Ok(Self {
            inner: Hydra::with_dimensions(p.rows, p.cols, cell),
            params: p,
        })
    }
}

impl Accumulator for HydraCms {
    type Item = Labeled<i64>;

    /// The key arrives already `;`-joined, which is the format the library
    /// splits on. Nothing is formatted here: the fan-out over label subsets
    /// happens inside `Hydra::update`, and is the cost this row exists to price.
    #[inline(always)]
    fn update(&mut self, r: &Labeled<i64>) {
        self.inner.update(&r.key, &DataInput::I64(r.value), None);
    }

    /// Cell-wise, and every cell is a Count-Min, so the fold is exact. The
    /// library's error cases are a dimension or counter-type mismatch, neither
    /// of which two sketches built from one `ParamSet` can hit.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so grid and cell shapes match");
        Ok(())
    }
}

impl SubpopFrequencyOps for HydraCms {
    type Value = i64;

    #[inline]
    fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
        self.inner
            .query_frequency(labels.to_vec(), &DataInput::I64(*value))
    }
}

impl MemoryFootprint for HydraCms {
    /// The grid holds `rows * cols` cells and every cell is a full Count-Min of
    /// `i32` counters, so the footprint is the product of both shapes.
    fn memory_bytes(&self) -> usize {
        let p = &self.params;
        p.rows * p.cols * p.cell_rows * p.cell_cols * std::mem::size_of::<i32>()
    }
}

// ---------- catalog identity ----------

impl BenchImpl for HydraCms {
    type Params = HydraParams;
    const IMPL: &'static str = "lib-cm";
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::config::SketchParams;

    fn built() -> HydraCms {
        HydraCms::init(&ParamSet::of(&HydraParams {
            rows: 3,
            cols: 64,
            cell_rows: 3,
            cell_cols: 256,
        }))
        .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> Labeled<i64> {
        Labeled {
            key: key.to_string(),
            value,
        }
    }

    /// The statistic is the frequency of a value *within* a subpopulation, and
    /// the coarse query must see every record whose first label matches, not
    /// only those whose full key does.
    #[test]
    fn a_coarse_query_covers_every_record_in_the_group() {
        let mut h = built();
        for r in [
            record("a;x", 10),
            record("a;y", 10),
            record("a;x", 20),
            record("b;x", 30),
        ] {
            h.update(&r);
        }
        // (a, 10) occurs twice, under two different second labels.
        assert_eq!(h.estimate_subpop_frequency(&["a"], &10), 2.0);
        assert_eq!(h.estimate_subpop_frequency(&["a"], &20), 1.0);
        assert_eq!(h.estimate_subpop_frequency(&["b"], &30), 1.0);
        // The full key is its own subpopulation and is stored too.
        assert_eq!(h.estimate_subpop_frequency(&["a", "x"], &10), 1.0);
    }

    /// A group that never occurred must estimate zero. This is the failure mode
    /// a grouped sketch has and an ungrouped one does not, so it is worth
    /// pinning even on a grid large enough to make collisions unlikely.
    #[test]
    fn an_absent_group_estimates_zero() {
        let mut h = built();
        h.update(&record("a;x", 10));
        assert_eq!(h.estimate_subpop_frequency(&["zzz"], &10), 0.0);
    }

    /// Count-Min is linear and the grid is cell-wise, so folding two shards is
    /// exact, not approximate.
    #[test]
    fn merging_shards_is_exact() {
        let (mut left, mut right) = (built(), built());
        for _ in 0..3 {
            left.update(&record("a;x", 10));
        }
        for _ in 0..4 {
            right.update(&record("a;x", 10));
        }
        left.merge(&right).expect("same ParamSet merges");
        assert_eq!(left.estimate_subpop_frequency(&["a"], &10), 7.0);
    }

    #[test]
    fn zero_dimensions_are_refused_by_name() {
        let bad = ParamSet::of(&HydraParams {
            rows: 3,
            cols: 0,
            cell_rows: 3,
            cell_cols: 256,
        });
        let Err(err) = HydraCms::init(&bad) else {
            panic!("a zero dimension must be refused, not built");
        };
        let err = err.to_string();
        assert!(err.contains("cols"), "error should name the field: {err}");
    }

    /// Footprint is the product of both shapes, which is the property that makes
    /// the two dimension pairs non-interchangeable.
    #[test]
    fn footprint_is_the_product_of_both_shapes() {
        let h = built();
        assert_eq!(h.memory_bytes(), 3 * 64 * 3 * 256 * 4);
    }

    #[test]
    fn canonical_params_build() {
        assert!(HydraCms::init(&ParamSet::of(&HydraParams::canonical())).is_ok());
    }
}
