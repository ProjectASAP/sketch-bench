//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022), a grid
//! of sketches answering per-subpopulation queries. The item is a record, and one
//! insert fans out into every non-empty label subset, so `d` labels cost
//! `2^d - 1` cell insertions; throughput is records per second. One file per cell
//! type, because the cell decides which statistic the grid answers.

use asap_sketchlib::input::HydraCounter;

use aqpbm_core::init::BuildError;

pub mod hydra_cms;
pub mod hydra_hll;
pub mod hydra_kll;

pub use hydra_cms::HydraCms;
pub use hydra_hll::HydraHll;
pub use hydra_kll::HydraKll;

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

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::config::{ParamSet, SketchParams};
    use aqpbm_core::init::InitSketch;
    use crate::params::{HydraCmsParams, HydraHllParams, HydraKllParams};

    #[test]
    fn canonical_params_build() {
        assert!(HydraCms::init(&ParamSet::of(&HydraCmsParams::canonical())).is_ok());
        assert!(HydraHll::init(&ParamSet::of(&HydraHllParams::canonical())).is_ok());
        assert!(HydraKll::init(&ParamSet::of(&HydraKllParams::canonical())).is_ok());
    }
}
