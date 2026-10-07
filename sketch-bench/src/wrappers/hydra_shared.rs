//! Hydra wrappers — `asap_sketchlib::Hydra` (Manousis et al., VLDB 2022). Their
//! item is a **record**; insert fans out into every non-empty label subset, so
//! `d` labels cost `2^d - 1` cells and throughput is records per second.

use crate::wrappers::BuildError;
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{DataInput, Hydra};

pub(crate) fn labels(group: &[String]) -> Vec<&str> {
    group.iter().map(String::as_str).collect()
}

/// The schema a grid holds until its first record names the label columns:
/// asap_sketchlib 0.3 fixes a Hydra's key columns at construction, and the
/// wrappers learn them from the stream.
const UNSET: &str = "__label_columns_unset__";

/// A grid whose key columns the first [`update`] sets.
pub(crate) fn new_hydra(rows: usize, cols: usize, cell: HydraCounter) -> Hydra {
    Hydra::with_schema(rows, cols, [UNSET], cell).expect("one label column is a valid schema")
}

fn unset(h: &Hydra) -> bool {
    h.schema() == [UNSET]
}

/// Records one `;`-joined record: one label value per column, positionally.
pub(crate) fn update(h: &mut Hydra, key: &str, value: &DataInput) {
    let parts: Vec<&str> = key.split(';').filter(|s| !s.is_empty()).collect();
    if unset(h) {
        let schema = (0..parts.len()).map(|i| format!("label{i}"));
        *h = Hydra::with_schema(h.row_num, h.col_num, schema, h.type_to_clone.clone())
            .expect("a record names at least one label");
    }
    h.update(&parts, value, None)
        .expect("every record carries one value per label column");
}

/// Asks `query` of the subpopulation `group` constrains: its values are the
/// leading label columns' (the scored subpopulations group by column 0), the
/// rest unconstrained. An empty grid answers 0.
pub(crate) fn query(h: &Hydra, group: &[&str], query: &HydraQuery) -> f64 {
    if unset(h) {
        return 0.0;
    }
    let mut key: Vec<Option<&str>> = group.iter().map(|v| Some(*v)).collect();
    key.resize(h.schema().len(), None);
    h.query_key(&key, query)
        .expect("a probe constrains the leading label columns")
}

/// Folds `other` into `acc`; a shard that saw no record (still unset) adds
/// nothing.
pub(crate) fn merge(acc: &mut Hydra, other: &Hydra) {
    if unset(other) {
        return;
    }
    if unset(acc) {
        *acc = other.clone();
        return;
    }
    acc.merge(other)
        .expect("both operands built from one ParamSet, so grid and cell shapes match");
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
