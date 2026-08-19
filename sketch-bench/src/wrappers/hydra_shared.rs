//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::wrappers::BuildError;
use asap_sketchlib::input::HydraCounter;

pub(crate) fn labels(group: &[String]) -> Vec<&str> {
    group.iter().map(String::as_str).collect()
}

/// outer grid is the one shape they have in common.
pub(crate) fn check_grid(rows: usize, cols: usize, algorithm: &str) -> Result<(), BuildError> {
    for (name, v) in [("rows", rows), ("cols", cols)] {
        if v == 0 {
            return Err(BuildError(format!("{algorithm}: {name} must be > 0")));
        }
    }
    Ok(())
}

/// Bytes the grid itself costs, on top of the counters inside the cells: every
/// cell is an enum around a sketch struct, plus the prototype `Hydra` clones
/// from. Separate from the counter bytes, so every row states both components.
pub(crate) fn grid_overhead_bytes(rows: usize, cols: usize) -> usize {
    (rows * cols + 1) * std::mem::size_of::<HydraCounter>()
}
