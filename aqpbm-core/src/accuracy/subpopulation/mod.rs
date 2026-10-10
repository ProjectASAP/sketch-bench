//! Ground truth for the grouped sketches: a statistic taken *within* one
//! subpopulation, one comparator per statistic.

mod cardinality;
mod cdf_error;
mod entropy;
mod frequency;
mod frequency_vector;
mod l1_norm;
mod l2_norm;
mod rank_error;
mod sum;

pub use cardinality::{SubpopCardTruth, SubpopCardinalityGT};
pub use cdf_error::SubpopCdfErrorGT;
pub use entropy::{entropy, SubpopEntropyGT};
pub use frequency::{SubpopFreqTruth, SubpopFrequencyGT};
pub use frequency_vector::SubpopVectorTruth;
pub use l1_norm::{l1_norm, SubpopL1NormGT};
pub use l2_norm::{l2_norm, SubpopL2NormGT};
pub use rank_error::{SubpopRankErrorGT, SubpopRankTruth};
pub use sum::SubpopSumGT;

use std::collections::BTreeMap;

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable};

use crate::accuracy::curve::percentile;
use crate::accuracy::GroupError;

pub type Group = Vec<String>;

/// A subpopulation as a grid is asked for it: one slot per label column up to
/// the last one grouped on, `Some` at the grouped columns and `None` at the
/// rest. Grouping on column 1 alone asks `[None, Some(v)]`, which a wrapper
/// can tell apart from grouping on column 0.
pub type GroupKey = Vec<Option<String>>;

pub(crate) fn group_labels<'a>(
    table: &'a GeneratedTable,
    columns: &[usize],
) -> Result<Vec<&'a str>, DataGenError> {
    if columns.is_empty() {
        return Err(DataGenError::BadParam(
            "a subpopulation is taken over at least one grouping column, and none were named"
                .into(),
        ));
    }
    let grouping: Vec<&[String]> = columns
        .iter()
        .map(|&i| String::column_slice(table.column(i)?))
        .collect::<Result<Vec<_>, DataGenError>>()?;
    let rows = table.row_num as usize;
    let mut flat = Vec::with_capacity(rows * columns.len());
    for r in 0..rows {
        for column in &grouping {
            flat.push(column[r].as_str());
        }
    }
    Ok(flat)
}

/// `group` holds one label per entry of `columns`, in that order.
fn owned(group: &[&str], columns: &[usize]) -> GroupKey {
    let mut key = vec![None; columns.iter().max().map_or(0, |last| last + 1)];
    for (label, &column) in group.iter().zip(columns) {
        key[column] = Some((*label).to_string());
    }
    key
}

/// How a group is named in `--per-group-out`: each grouped column's value
/// tagged with its column, as the grid's subkeys are (`label1:x`), with `\`,
/// `:` and `;` escaped. The same encoding as sketch-bench's
/// `polars_shared::subset_key`, which this crate cannot depend on.
fn rendered(key: &GroupKey) -> String {
    let mut out = String::new();
    for (j, label) in key.iter().enumerate() {
        let Some(label) = label else { continue };
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(&format!("label{j}:"));
        for ch in label.chars() {
            if matches!(ch, '\\' | ':' | ';') {
                out.push('\\');
            }
            out.push(ch);
        }
    }
    out
}

/// One [`GroupError`] per group, heaviest first (ties on the key), so the
/// per-group file reads the same across runs.
fn group_errors(errors: impl Iterator<Item = (GroupKey, u64, Option<f64>)>) -> Vec<GroupError> {
    let mut out: Vec<GroupError> = errors
        .map(|(key, n_q, error)| GroupError {
            group: rendered(&key),
            n_q,
            error,
        })
        .collect();
    out.sort_by(|a, b| b.n_q.cmp(&a.n_q).then_with(|| a.group.cmp(&b.group)));
    out
}

/// The per-group summary every grouped comparator reports, in its own metric:
/// `err_mean` / `err_p50` / `err_p90` / `err_max` over the scored groups (none
/// written when no group was scored), `groups_scored`, and the stream's shape
/// against which a group's error is read: `schema_width` (`d`, the label
/// columns before the value), `records` (`N`) and `fanned_mass`
/// (`N·(2^d − 1)`, every subset a record is inserted into).
fn write_groups(
    groups: &[GroupError],
    records: u64,
    schema_width: usize,
    metrics: &mut BTreeMap<String, f64>,
) {
    let mut errs: Vec<f64> = groups.iter().filter_map(|g| g.error).collect();
    errs.sort_by(f64::total_cmp);
    if let Some(&max) = errs.last() {
        metrics.insert(
            "err_mean".into(),
            errs.iter().sum::<f64>() / errs.len() as f64,
        );
        metrics.insert("err_p50".into(), percentile(&errs, 0.5));
        metrics.insert("err_p90".into(), percentile(&errs, 0.9));
        metrics.insert("err_max".into(), max);
    }
    metrics.insert("groups_scored".into(), errs.len() as f64);
    metrics.insert("schema_width".into(), schema_width as f64);
    metrics.insert("records".into(), records as f64);
    let subsets = 2f64.powi(schema_width as i32) - 1.0;
    metrics.insert("fanned_mass".into(), records as f64 * subsets);
}

/// key1 ∈ {a, b}, key2 ∈ {x, y}; the value column is what gets counted.
#[cfg(test)]
fn records() -> GeneratedTable {
    use crate::accuracy::table_of;
    use aqpbm_datagen::ColumnData;

    let rows = [
        ("a", "x", 10),
        ("a", "y", 10),
        ("a", "x", 20),
        ("b", "x", 30),
        ("b", "y", 30),
        ("b", "y", 30),
    ];
    table_of(
        &["key1", "key2", "value"],
        vec![
            ColumnData::String(rows.iter().map(|r| r.0.to_string()).collect()),
            ColumnData::String(rows.iter().map(|r| r.1.to_string()).collect()),
            ColumnData::Int64(rows.iter().map(|r| r.2).collect()),
        ],
    )
}

#[cfg(test)]
fn null_over<G>(gt: &G, table: &GeneratedTable) -> std::collections::BTreeMap<String, f64>
where
    G: crate::accuracy::GroundTruth<Probe = GroupKey, Answer = f64>,
{
    crate::accuracy::score_with(gt, &|_: &mut (), _: &GroupKey| 0.0, &mut (), table)
}

#[cfg(test)]
fn exact_over<G>(gt: &G, table: &GeneratedTable) -> std::collections::BTreeMap<String, f64>
where
    G: crate::accuracy::GroundTruth<Truth = SubpopVectorTruth, Probe = GroupKey, Answer = f64>,
{
    let mut exact = gt
        .truth(table)
        .expect("the test's table matches its comparator")
        .exact;
    crate::accuracy::score_with(
        gt,
        &|held: &mut std::collections::HashMap<GroupKey, f64>, p: &GroupKey| {
            held.get(p).copied().unwrap_or(0.0)
        },
        &mut exact,
        table,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label holding the separators is escaped as the grid's subkeys are,
    /// so it cannot be read as two columns.
    #[test]
    fn rendered_keys_escape_the_separators() {
        let key = vec![None, Some(r"x:y;z\".to_string()), Some(String::new())];
        assert_eq!(rendered(&key), r"label1:x\:y\;z\\;label2:");
    }
}
