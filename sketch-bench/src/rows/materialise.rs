use super::*;

// ---------- what every row does with what it named ----------

/// Materialise at the item type this row ingests: the value column, read at
/// the row's own width. The stream is moved into an `Rc` rather than copied:
/// every measurement of this row reads the same one.
pub(super) fn peel<T: ColumnItem>(
    columns: &ColumnRoles,
    table: GeneratedTable,
) -> Result<Rc<Vec<T>>, RunError> {
    let column = table.into_column(columns.value)?;
    Ok(Rc::new(T::from_column(column)?))
}

/// The same, for the grouped rows: the label columns joined
/// with `;` into one key, paired with the value. The join happens here rather
/// than on the insert path, so a wrapper feeding a library that takes `"a;b"`
/// pays nothing for it per item.
pub(super) fn peel_labeled<V: ColumnItem>(
    columns: &ColumnRoles,
    table: GeneratedTable,
) -> Result<Rc<Vec<(String, V)>>, RunError> {
    let (labels, values) = label_columns::<V>(columns, table)?;
    let mut items = Vec::with_capacity(values.len());
    for (row, value) in values.into_iter().enumerate() {
        let mut key = String::new();
        for (column, labels) in labels.iter().enumerate() {
            if column > 0 {
                key.push(';');
            }
            key.push_str(&labels[row]);
        }
        items.push((key, value));
    }
    Ok(Rc::new(items))
}

#[allow(clippy::type_complexity)]
/// The same records with the label values kept apart, as ASAPQuery's
/// `KeyByLabelValues` holds them.
pub(super) fn peel_grouped<V: ColumnItem>(
    columns: &ColumnRoles,
    table: GeneratedTable,
) -> Result<Rc<Vec<(Vec<String>, V)>>, RunError> {
    let (labels, values) = label_columns::<V>(columns, table)?;
    let items = values
        .into_iter()
        .enumerate()
        .map(|(row, value)| (labels.iter().map(|c| c[row].clone()).collect(), value))
        .collect();
    Ok(Rc::new(items))
}

/// The label columns, and the value column at the row's width.
#[allow(clippy::type_complexity)]
fn label_columns<V: ColumnItem>(
    columns: &ColumnRoles,
    table: GeneratedTable,
) -> Result<(Vec<Vec<String>>, Vec<V>), RunError> {
    if columns.labels.is_empty() {
        return Err(RunError::Sketch(
            "this row ingests labelled records, so it needs at least one label \
             column besides the value column"
                .into(),
        ));
    }
    let titles = table.column_title.clone();
    let mut data = taken(table);
    let values = V::from_column(take(&mut data, columns.value)?).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. The value column's data_type has to be the \
             row's item type",
            titles[columns.value],
        ))
    })?;
    let labels: Vec<Vec<String>> = columns
        .labels
        .iter()
        .map(|&i| {
            String::from_column(take(&mut data, i)?).map_err(|e| {
                RunError::Sketch(format!(
                    "label column '{}': {e}. Label columns are rendered as text, \
                     so their data_type has to be `string`",
                    titles[i],
                ))
            })
        })
        .collect::<Result<_, RunError>>()?;
    Ok((labels, values))
}

pub(super) fn peel_keyed<K: ColumnItem>(
    columns: &ColumnRoles,
    table: GeneratedTable,
) -> Result<Rc<Vec<(K, i64)>>, RunError> {
    let key = key_column(columns)?;
    let titles = table.column_title.clone();
    let mut data = taken(table);
    let values = i64::from_column(take(&mut data, columns.value)?).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. A keyed row weights its keys with `i64`",
            titles[columns.value],
        ))
    })?;
    let keys = K::from_column(take(&mut data, key)?).map_err(|e| {
        RunError::Sketch(format!(
            "key column '{}': {e}. A keyed row hashes its keys at the width \
             `--dtype` names",
            titles[key],
        ))
    })?;
    Ok(Rc::new(keys.into_iter().zip(values).collect()))
}

fn taken(table: GeneratedTable) -> Vec<Option<ColumnData>> {
    table.into_columns().into_iter().map(Some).collect()
}

fn take(data: &mut [Option<ColumnData>], i: usize) -> Result<ColumnData, RunError> {
    let held = data.len();
    data.get_mut(i).and_then(Option::take).ok_or_else(|| {
        RunError::Sketch(format!(
            "column {i} was asked for, but the table holds {held} or it was named twice"
        ))
    })
}

/// The ordered rows build at every numeric width and at neither `string`: a
/// quantile cell stores what it can compare, and text is the one thing the
/// generator renders that carries no order to compare on. The cardinality rows
/// read every width, so they never reach this. The rows that read no width at
/// all are handed the data the frontend generated, and materialising it at the
/// row's own item type is what catches a stream they cannot ingest.
pub(super) fn no_build_at(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} builds at i64, u64 or f64; --dtype {} is none of them",
        req.variant,
        req.library,
        got.name(),
    ))
}
