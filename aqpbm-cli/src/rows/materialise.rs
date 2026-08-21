use super::*;

// ---------- what every row does with what it named ----------

/// Materialise at the item type this row ingests: the value column, read at
/// the row's own width. The stream is moved into an `Rc` rather than copied:
/// every measurement of this row reads the same one.
pub(super) fn peel<T: ColumnItem>(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<T>>, RunError> {
    let column = table.into_column(value_column(description))?;
    Ok(Rc::new(T::from_column(column)?))
}

/// The same, for the grouped rows: the columns before the value column joined
/// with `;` into one key, paired with the value. The join happens here rather
/// than on the insert path, so a wrapper feeding a library that takes `"a;b"`
/// pays nothing for it per item.
pub(super) fn peel_labeled<V: ColumnItem>(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<(String, V)>>, RunError> {
    let value_column = value_column(description);
    if value_column == 0 {
        return Err(RunError::Sketch(format!(
            "this row ingests labelled records, so it needs at least one label \
             column before the value column; the description has {} column(s)",
            description.column_spec.len(),
        )));
    }
    let titles = table.column_title.clone();
    let mut columns = table.into_columns();
    if value_column >= columns.len() {
        return Err(RunError::Sketch(format!(
            "column {value_column} was asked for, but the table holds {}",
            columns.len(),
        )));
    }
    let values = V::from_column(columns.remove(value_column)).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. The value column's data_type has to be the \
             row's item type",
            titles[value_column],
        ))
    })?;
    columns.truncate(value_column);
    let labels: Vec<Vec<String>> = columns
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            String::from_column(c).map_err(|e| {
                RunError::Sketch(format!(
                    "label column '{}': {e}. Label columns are rendered as text, \
                     so their data_type has to be `string`",
                    titles[i],
                ))
            })
        })
        .collect::<Result<_, RunError>>()?;

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

pub(super) fn peel_keyed(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<(u64, i64)>>, RunError> {
    let value_column = value_column(description);
    if value_column == 0 {
        return Err(RunError::Sketch(format!(
            "this row ingests keyed records, so it needs a key column before the \
             value column; the description has {} column(s)",
            description.column_spec.len(),
        )));
    }
    let titles = table.column_title.clone();
    let mut columns = table.into_columns();
    if value_column >= columns.len() {
        return Err(RunError::Sketch(format!(
            "column {value_column} was asked for, but the table holds {}",
            columns.len(),
        )));
    }
    let values = i64::from_column(columns.remove(value_column)).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. A keyed row weights its keys with `i64`",
            titles[value_column],
        ))
    })?;
    let keys = u64::from_column(columns.remove(KEYED_KEY_COLUMN)).map_err(|e| {
        RunError::Sketch(format!(
            "key column '{}': {e}. A keyed row hashes its keys as `u64`",
            titles[KEYED_KEY_COLUMN],
        ))
    })?;
    Ok(Rc::new(keys.into_iter().zip(values).collect()))
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
