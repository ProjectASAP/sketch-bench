//! `hydra-cms` — Hydra over Count-Min cells. A cell counts occurrences of a
//! value inside a group, which is the statistic a counter array can answer and
//! `hll`'s cannot: it cannot report the size of the group itself.

use asap_sketchlib::input::HydraCounter;
use asap_sketchlib::{CountMin, DataInput, FastPath, Hydra, Vector2D};

use aqpbm_core::accuracy::SubpopFrequencyOps;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;

use super::{check_grid, grid_overhead_bytes};
use crate::params::HydraCmsParams;

/// Hydra over Count-Min cells.
pub struct HydraCms {
    inner: Hydra,
    params: HydraCmsParams,
}

impl InitSketch for HydraCms {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HydraCmsParams = config.parse()?;
        check_grid(p.rows, p.cols, "hydra-cms")?;
        for (name, v) in [("cell_rows", p.cell_rows), ("cell_cols", p.cell_cols)] {
            if v == 0 {
                return Err(BuildError(format!("hydra-cms: {name} must be > 0")));
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
    /// `i32` counters, so the counter term is the product of both shapes.
    fn memory_bytes(&self) -> usize {
        let p = &self.params;
        p.rows * p.cols * p.cell_rows * p.cell_cols * std::mem::size_of::<i32>()
            + grid_overhead_bytes(p.rows, p.cols)
    }
}

impl BenchImpl for HydraCms {
    type Params = HydraCmsParams;
    const IMPL: &'static str = "lib";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> HydraCms {
        HydraCms::init(&ParamSet::of(&HydraCmsParams {
            rows: 3,
            cols: 64,
            cell_rows: 3,
            cell_cols: 256,
        }))
        .expect("canonical dimensions build")
    }

    pub(super) fn record(key: &str, value: i64) -> Labeled<i64> {
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
        let bad = ParamSet::of(&HydraCmsParams {
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

    /// Footprint is the product of both shapes plus the grid's own cells, which
    /// is the property that makes the two dimension pairs non-interchangeable.
    #[test]
    fn footprint_is_the_product_of_both_shapes() {
        let h = built();
        let counters = 3 * 64 * 3 * 256 * 4;
        assert_eq!(h.memory_bytes(), counters + grid_overhead_bytes(3, 64));
        // The counters still dominate, so the overhead term must not be what
        // the number is mostly made of.
        assert!(h.memory_bytes() < counters * 2);
    }
}
