//! `hydra-kll` — Hydra over KLL cells, answering the ordered statistic inside a
//! group. The cell is the same `asap_sketchlib::KLL` the `kll-*` rows hold, so
//! its `k` bounds are theirs.

use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra, KLL};

use aqpbm_core::accuracy::SubpopQuantileOps;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::workload::Labeled;

use super::{check_grid, grid_overhead_bytes};
use crate::params::HydraKllParams;

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
