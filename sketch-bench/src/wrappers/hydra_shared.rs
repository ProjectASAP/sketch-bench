//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::wrappers::BuildError;
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra};

pub(crate) fn labels(group: &[Option<String>]) -> Vec<Option<&str>> {
    group.iter().map(Option::as_deref).collect()
}

/// The label columns of a `;`-joined record stream, which every record must
/// share: a record of another width is a [`BuildError`] naming `variant` and
/// both widths. An empty label is a column of its own.
pub(crate) fn label_columns<V>(items: &[(String, V)], variant: &str) -> Result<usize, BuildError> {
    let width = |key: &str| key.split(';').count();
    let Some((first, _)) = items.first() else {
        return Ok(1);
    };
    let columns = width(first);
    match items.iter().find(|(key, _)| width(key) != columns) {
        Some((key, _)) => Err(BuildError(format!(
            "{variant}: record {key:?} has {} label(s), the stream's first {columns}",
            width(key)
        ))),
        None => Ok(columns),
    }
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
    let schema = (0..label_columns).map(|i| format!("label{i}"));
    Hydra::with_schema(rows, cols, schema, cell).map_err(|e| BuildError(format!("{variant}: {e}")))
}

/// Records one `;`-joined record, one value per key column in place. The
/// build already refused a stream of mixed widths; this is the backstop.
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

/// Asks `query` of the subpopulation `group` constrains: a value at each
/// grouped column, `None` (unconstrained) at the rest and past its end, so any
/// non-empty subset of the key columns can be asked. A probe wider than the
/// grid is refused.
pub(crate) fn query(h: &Hydra, group: &[Option<&str>], query: &HydraQuery, variant: &str) -> f64 {
    let width = h.schema().len();
    if group.len() > width {
        panic!(
            "{variant}: probe {group:?} names {} label(s), the grid {width}",
            group.len()
        );
    }
    let key: Vec<Option<&str>> = (0..width)
        .map(|i| group.get(i).copied().flatten())
        .collect();
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
