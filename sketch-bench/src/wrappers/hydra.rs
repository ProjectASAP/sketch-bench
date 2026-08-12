//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022), a grid
//! of sketches answering per-subpopulation queries. The item is a record, and one
//! insert fans out into every non-empty label subset, so `d` labels cost
//! `2^d - 1` cell insertions; throughput is records per second. The cell type is
//! on the algorithm axis because it decides which statistic the grid answers.

use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{CountMin, DataInput, FastPath, Hydra, HyperLogLog, Vector2D, KLL};

use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::accuracy::{SubpopCardinalityOps, SubpopFrequencyOps, SubpopQuantileOps};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;

use crate::params::{HydraCmsParams, HydraHllParams, HydraKllParams};

/// Refuse a zero grid dimension by name. Shared by all three rows, because the
/// outer grid is the one shape they have in common.
fn check_grid(rows: usize, cols: usize, algorithm: &str) -> Result<(), BuildError> {
    for (name, v) in [("rows", rows), ("cols", cols)] {
        if v == 0 {
            return Err(BuildError(format!("{algorithm}: {name} must be > 0")));
        }
    }
    Ok(())
}

/// Bytes the grid itself costs, on top of the counters inside the cells: every
/// cell is an enum around a sketch struct, and `Hydra` keeps one more of them
/// as the prototype it clones into new cells.
///
/// Reported separately from the counter bytes so each row's footprint states
/// the same two components. #75 records that leaving this out is a fixed
/// under-report.
fn grid_overhead_bytes(rows: usize, cols: usize) -> usize {
    (rows * cols + 1) * std::mem::size_of::<HydraCounter>()
}

// ---------- hydra-cms: subpopulation frequency ----------

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

// ---------- hydra-hll: subpopulation cardinality ----------

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

// ---------- hydra-kll: subpopulation quantile ----------

// Level count and decay of an `asap_sketchlib::KLL`. Both are private constants
// in the library, reproduced here because a cell's footprint is a function of
// them and the library exposes no accessor for its own capacity.
const KLL_MAX_LEVELS: usize = 61;
const KLL_CAPACITY_DECAY: f64 = 2.0 / 3.0;
/// The `m` the library's `init_kll` passes, its minimum level capacity, and the
/// floor it silently raises a smaller `k` to. Same cell as the `kll` rows, so
/// the bound is theirs: see [`crate::wrappers::kll::LIB_K_MIN`].
const KLL_MIN_LEVEL: usize = crate::wrappers::kll::LIB_K_MIN as usize;
/// The library clamps `k` to this before sizing, so a larger `k` buys nothing.
const KLL_MAX_CACHEABLE_K: usize = crate::wrappers::kll::LIB_K_MAX as usize;

/// Retained slots one KLL cell allocates at construction.
///
/// Worth knowing for #56, which assumed a KLL cell is a variable-size heap
/// structure with no analytic footprint: in this library it is not. `KLL::init`
/// allocates `items` as a boxed slice of this length once and never grows it,
/// so a `hydra-kll` footprint is as analytic as a `hydra-cms` one.
///
/// This is a line-for-line copy of the library's private
/// `compute_max_capacity`, which makes it a claim about `asap_sketchlib` 0.2.2
/// and not a bound that holds by construction. Nothing in the library's public
/// API reports the allocation, so the test beside it can only pin this
/// reproduction and would not notice the library diverging from it. Read the
/// number the way `kll`'s own rows ask theirs to be read: a derived figure to
/// compare against `heap_bytes_net`, which is the measured one.
fn kll_cell_slots(k: u32) -> usize {
    // `init_internal` normalises before sizing: `m` floors `k`, and `k` is
    // capped. Reproduced so an out-of-range `k` reports the footprint the
    // library actually allocates.
    let m = KLL_MIN_LEVEL;
    let k = (k as usize).max(m).min(KLL_MAX_CACHEABLE_K) as f64;
    let mut total = 0usize;
    let mut scale = 1.0f64;
    for _ in 0..KLL_MAX_LEVELS {
        total += (k * scale).ceil().max(m as f64) as usize;
        scale *= KLL_CAPACITY_DECAY;
    }
    total
}

/// Hydra over KLL cells.
pub struct HydraKll {
    inner: Hydra,
    params: HydraKllParams,
}

impl InitSketch for HydraKll {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HydraKllParams = config.parse()?;
        check_grid(p.rows, p.cols, "hydra-kll")?;
        // The cell is the same `asap_sketchlib::KLL` the `kll-*` rows hold, and
        // it clamps `k` to its own range without saying so. Refuse here for the
        // same reason those rows do: outside the range the grid would be built
        // at a `cell_k` the record does not name. Below the floor every value
        // gave one sketch at `cell_k = 8`; above the ceiling every value gave
        // one sketch at 26602, while the footprint column kept climbing.
        if !(crate::wrappers::kll::LIB_K_MIN..=crate::wrappers::kll::LIB_K_MAX).contains(&p.cell_k)
        {
            return Err(BuildError(format!(
                "hydra-kll: cell_k={} outside [{}, {}]; the library clamps to that range",
                p.cell_k,
                crate::wrappers::kll::LIB_K_MIN,
                crate::wrappers::kll::LIB_K_MAX
            )));
        }
        let cell = HydraCounter::KLL(KLL::init_kll(p.cell_k as i32));
        Ok(Self {
            inner: Hydra::with_dimensions(p.rows, p.cols, cell),
            params: p,
        })
    }
}

impl Accumulator for HydraKll {
    /// `f64`, because the ordered statistic is over a numeric value column and
    /// the library's cell is a `KLL<f64>`. `Labeled<V>` is already generic, so
    /// this needs nothing from the core.
    type Item = Labeled<f64>;

    #[inline(always)]
    fn update(&mut self, r: &Labeled<f64>) {
        self.inner.update(&r.key, &DataInput::F64(r.value), None);
    }

    /// KLL merges by concatenating levels and recompacting, so unlike the other
    /// two rows this fold is lossy. That is the measurement, not a defect: the
    /// merge pass exists to price it.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so grid and cell shapes match");
        Ok(())
    }
}

impl SubpopQuantileOps for HydraKll {
    /// `HydraQuery::Quantile` and not `Cdf`: the comparator asks for the value
    /// at a rank, which is what rank error is defined over. The `Cdf` variant
    /// answers the inverse question.
    #[inline]
    fn estimate_subpop_quantile(&self, labels: &[&str], phi: f64) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Quantile(phi))
    }
}

impl MemoryFootprint for HydraKll {
    /// Retained slots per cell times the grid area, plus the level index every
    /// cell carries. Analytic because the cell allocates once, see
    /// [`kll_cell_slots`].
    fn memory_bytes(&self) -> usize {
        let p = &self.params;
        let per_cell = kll_cell_slots(p.cell_k) * std::mem::size_of::<f64>()
            + (KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
        p.rows * p.cols * per_cell + grid_overhead_bytes(p.rows, p.cols)
    }
}

impl BenchImpl for HydraKll {
    type Params = HydraKllParams;
    const IMPL: &'static str = "lib";
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::config::SketchParams;

    fn built() -> HydraCms {
        HydraCms::init(&ParamSet::of(&HydraCmsParams {
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

    #[test]
    fn canonical_params_build() {
        assert!(HydraCms::init(&ParamSet::of(&HydraCmsParams::canonical())).is_ok());
        assert!(HydraHll::init(&ParamSet::of(&HydraHllParams::canonical())).is_ok());
        assert!(HydraKll::init(&ParamSet::of(&HydraKllParams::canonical())).is_ok());
    }

    // ---------- hydra-hll ----------

    fn built_hll() -> HydraHll {
        HydraHll::init(&ParamSet::of(&HydraHllParams { rows: 3, cols: 64 }))
            .expect("canonical dimensions build")
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

    // ---------- hydra-kll ----------

    fn built_kll() -> HydraKll {
        HydraKll::init(&ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 200,
        }))
        .expect("canonical dimensions build")
    }

    fn frecord(key: &str, value: f64) -> Labeled<f64> {
        Labeled {
            key: key.to_string(),
            value,
        }
    }

    /// The statistic is ordered and taken inside a group, so the median of one
    /// group must not be pulled by another group's values.
    #[test]
    fn kll_quantiles_are_taken_inside_the_group() {
        let mut h = built_kll();
        for v in 1..=101 {
            h.update(&frecord("a;x", v as f64));
        }
        for _ in 0..500 {
            h.update(&frecord("b;x", 10_000.0));
        }
        let median = h.estimate_subpop_quantile(&["a"], 0.5);
        assert!(
            (1.0..=101.0).contains(&median),
            "group `a` spans 1..=101, median estimated {median}"
        );
    }

    /// `k` is well past the group size here, so the cell retains everything and
    /// the answer is exact. Pinned because it is what makes a rank error at a
    /// larger workload attributable to compaction and not to the grid.
    #[test]
    fn kll_is_exact_below_k() {
        let mut h = built_kll();
        for v in 1..=101 {
            h.update(&frecord("a;x", v as f64));
        }
        assert_eq!(h.estimate_subpop_quantile(&["a"], 0.0), 1.0);
        assert_eq!(h.estimate_subpop_quantile(&["a"], 1.0), 101.0);
    }

    /// The library allocates a cell's retained slots once at construction, so
    /// the footprint is analytic.
    ///
    /// This pins the reproduction, not the library: nothing public reports the
    /// allocation, so a library change to `compute_max_capacity` would pass
    /// here and silently move every `hydra-kll` memory number. The value below
    /// is hand-derived from the decay series, so at least it is not this
    /// function checking itself.
    #[test]
    fn kll_cell_capacity_matches_the_library_shape() {
        // ceil(200 * (2/3)^i) for i in 0..8 is 200, 134, 89, 60, 40, 27, 18, 12
        // summing to 580; every level from 8 up is floored at m = 8, and there
        // are 53 of them, adding 424.
        assert_eq!(kll_cell_slots(200), 580 + 424);
        // Monotone in k, and never below the floor times the level count.
        assert!(kll_cell_slots(400) > kll_cell_slots(200));
        assert_eq!(kll_cell_slots(1), KLL_MIN_LEVEL * KLL_MAX_LEVELS);
        // Past the library's clamp, a larger k buys no more slots.
        assert_eq!(
            kll_cell_slots(KLL_MAX_CACHEABLE_K as u32),
            kll_cell_slots(KLL_MAX_CACHEABLE_K as u32 + 5_000)
        );
    }

    #[test]
    fn kll_zero_cell_k_is_refused_by_name() {
        let bad = ParamSet::of(&HydraKllParams {
            rows: 3,
            cols: 64,
            cell_k: 0,
        });
        let Err(err) = HydraKll::init(&bad) else {
            panic!("a zero cell_k must be refused, not built");
        };
        assert!(
            err.to_string().contains("cell_k"),
            "error should name the field: {err}"
        );
    }
}
