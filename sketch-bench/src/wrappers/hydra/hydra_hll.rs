//! `hydra-hll` — Hydra over HyperLogLog cells. A cell counts *distinct* values
//! inside a group, the statistic the Count-Min cell structurally cannot reach.

use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra, HyperLogLog};

use aqpbm_core::accuracy::SubpopCardinalityOps;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;

use super::{check_grid, grid_overhead_bytes};
use crate::params::HydraHllParams;

/// Registers in one `HyperLogLog<ErtlMLE>` cell. The library fixes the cell at
/// `HyperLogLogP14`, so this is `2^14` and not a parameter. Named here because
/// the footprint depends on it and nothing in the params struct states it.
const HLL_CELL_REGISTERS: usize = 1 << 14;

/// Hydra over HyperLogLog cells.
pub struct HydraHll {
    inner: Hydra,
    params: HydraHllParams,
}

impl InitSketch for HydraHll {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HydraHllParams = config.parse()?;
        check_grid(p.rows, p.cols, "hydra-hll")?;
        // Named through the `ErtlMLE` impl explicitly: `HyperLogLog` is a type
        // alias over the variant, so `new()` is ambiguous between the
        // estimators the alias can carry. The enum fixes this one.
        let cell = HydraCounter::HLL(HyperLogLog::<asap_sketchlib::ErtlMLE>::new());
        Ok(Self {
            inner: Hydra::with_dimensions(p.rows, p.cols, cell),
            params: p,
        })
    }
}

impl Accumulator for HydraHll {
    type Item = Labeled<i64>;

    /// Same fan-out as the Count-Min row, and the same cost per record. What
    /// changes is what a cell does with the value: a HyperLogLog folds it into
    /// registers, so repeats after the first are free in state and not in time.
    #[inline(always)]
    fn update(&mut self, r: &Labeled<i64>) {
        self.inner.update(&r.key, &DataInput::I64(r.value), None);
    }

    /// Register-wise maximum, so the fold is exact for the same reason the
    /// Count-Min one is: a HyperLogLog is mergeable without loss.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so grid and cell shapes match");
        Ok(())
    }
}

impl SubpopCardinalityOps for HydraHll {
    #[inline]
    fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Cardinality)
    }
}

impl MemoryFootprint for HydraHll {
    /// One byte per register per cell. The cell is fixed-shape, so unlike the
    /// Count-Min row there is no cell parameter in this product.
    fn memory_bytes(&self) -> usize {
        let p = &self.params;
        p.rows * p.cols * HLL_CELL_REGISTERS + grid_overhead_bytes(p.rows, p.cols)
    }
}

impl BenchImpl for HydraHll {
    type Params = HydraHllParams;
    const IMPL: &'static str = "lib";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_hll() -> HydraHll {
        HydraHll::init(&ParamSet::of(&HydraHllParams { rows: 3, cols: 64 }))
            .expect("canonical dimensions build")
    }

    fn record(key: &str, value: i64) -> Labeled<i64> {
        Labeled {
            key: key.to_string(),
            value,
        }
    }

    /// The statistic is the *size* of the group, so repeats of one value must
    /// not raise it. This is the property that separates this row from the
    /// Count-Min one, which would answer 3 here.
    #[test]
    fn hll_counts_distinct_values_not_occurrences() {
        let mut h = built_hll();
        for _ in 0..3 {
            h.update(&record("a;x", 10));
        }
        h.update(&record("a;y", 20));
        // Two distinct values under label `a`, seen four times.
        let est = h.estimate_subpop_cardinality(&["a"]);
        assert!(
            (est - 2.0).abs() < 0.5,
            "two distinct values under `a`, estimated {est}"
        );
    }

    #[test]
    fn hll_absent_group_estimates_zero() {
        let mut h = built_hll();
        h.update(&record("a;x", 10));
        assert_eq!(h.estimate_subpop_cardinality(&["zzz"]), 0.0);
    }

    /// A HyperLogLog is register-wise mergeable, so two shards fold without
    /// double counting the value they share.
    #[test]
    fn hll_merging_shards_does_not_double_count() {
        let (mut left, mut right) = (built_hll(), built_hll());
        left.update(&record("a;x", 10));
        left.update(&record("a;x", 20));
        right.update(&record("a;x", 20));
        right.update(&record("a;x", 30));
        left.merge(&right).expect("same ParamSet merges");
        let est = left.estimate_subpop_cardinality(&["a"]);
        assert!(
            (est - 3.0).abs() < 0.5,
            "three distinct values across both shards, estimated {est}"
        );
    }

    /// The cell is fixed-shape, so the footprint is the grid area times a
    /// constant, and no construction parameter can move it.
    #[test]
    fn hll_footprint_is_grid_area_times_a_fixed_cell() {
        let h = built_hll();
        assert_eq!(
            h.memory_bytes(),
            3 * 64 * HLL_CELL_REGISTERS + grid_overhead_bytes(3, 64)
        );
    }
}
