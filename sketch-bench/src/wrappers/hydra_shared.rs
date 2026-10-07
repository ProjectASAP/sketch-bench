//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::wrappers::BuildError;
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra};

pub(crate) fn labels(group: &[String]) -> Vec<&str> {
    group.iter().map(String::as_str).collect()
}

/// The label columns of a `;`-joined record stream: its first record's, as
/// every record of one table carries the same columns. An empty label is a
/// column of its own.
pub(crate) fn label_columns<V>(items: &[(String, V)]) -> usize {
    items.first().map_or(1, |(key, _)| key.split(';').count())
}

/// A grid over `label_columns` key columns, as asap_sketchlib 0.3 fixes
/// them at construction.
pub(crate) fn new_hydra(
    rows: usize,
    cols: usize,
    label_columns: usize,
    cell: HydraCounter,
    variant: &str,
) -> Result<Hydra, BuildError> {
    if label_columns == 0 {
        return Err(BuildError(format!(
            "{variant}: a record needs at least one label column"
        )));
    }
    let schema = (0..label_columns).map(|i| format!("label{i}"));
    Hydra::with_schema(rows, cols, schema, cell).map_err(|e| BuildError(format!("{variant}: {e}")))
}

/// Records one `;`-joined record, one value per key column in place. A
/// record of another width is refused.
pub(crate) fn update(h: &mut Hydra, key: &str, value: &DataInput, variant: &str) {
    let parts: Vec<&str> = key.split(';').collect();
    if let Err(e) = h.update(&parts, value, None) {
        panic!(
            "{variant}: record {key:?} has {} label(s), the grid {}: {e}",
            parts.len(),
            h.schema().len()
        );
    }
}

/// Asks `query` of the subpopulation `group` constrains: its values are the
/// leading key columns' (the scored subpopulations group by column 0), the
/// rest unconstrained. A probe wider than the grid is refused.
pub(crate) fn query(h: &Hydra, group: &[&str], query: &HydraQuery, variant: &str) -> f64 {
    let width = h.schema().len();
    if group.len() > width {
        panic!(
            "{variant}: probe {group:?} names {} label(s), the grid {width}",
            group.len()
        );
    }
    let key: Vec<Option<&str>> = (0..width).map(|i| group.get(i).copied()).collect();
    h.query_key(&key, query)
        .unwrap_or_else(|e| panic!("{variant}: probe {group:?}: {e}"))
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
