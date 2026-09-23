use std::rc::Rc;
use std::sync::OnceLock;

use aqpbm_datagen::table::TableDescription;

use asap_frontend_sql::{lower_sql, SqlCatalog};
use asap_types::pre_asap::schema::{Column, DataType, Schema};
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;

use crate::types::EvalError;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread tokio runtime needs no OS resources to start")
    })
}

pub fn table_name_from_path(path: &std::path::Path) -> Result<String, EvalError> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            EvalError::Planning(format!(
                "{} has no file stem to name a table after; pass --table",
                path.display()
            ))
        })
}

fn column_type(
    label: &str,
    data_type: &str,
    sql_type: Option<&str>,
) -> Result<DataType, EvalError> {
    match (data_type, sql_type) {
        ("i64", Some("timestamp_ms")) => Ok(DataType::Timestamp),
        ("i64" | "u64", None) => Ok(DataType::Int64),
        ("f64", None) => Ok(DataType::Float64),
        ("string", None) => Ok(DataType::Utf8),
        (_, Some(declared)) => Err(EvalError::Planning(format!(
            "column {label:?}: sql_type {declared:?} over data_type {data_type:?} has no SQL type"
        ))),
        _ => Err(EvalError::Planning(format!(
            "column {label:?}: data_type {data_type:?} has no SQL type"
        ))),
    }
}

pub fn catalog_from_spec(
    table: &str,
    description: &TableDescription,
) -> Result<SqlCatalog, EvalError> {
    let mut columns = Vec::with_capacity(description.column_spec.len());
    let mut time_index = None;
    for (position, spec) in description.column_spec.iter().enumerate() {
        let label = description.column_label.get(position).ok_or_else(|| {
            EvalError::Planning(format!("column {position} has a spec but no label"))
        })?;
        let dtype = column_type(label, &spec.data_type, spec.sql_type.as_deref())?;
        if dtype == DataType::Timestamp && time_index.is_none() {
            time_index = Some(position);
        }
        columns.push(Column::new(label, dtype, false));
    }

    let schema = match time_index {
        Some(index) => Schema::with_time_index(columns, index, Vec::new()),
        None => Schema::new(columns),
    };
    Ok(SqlCatalog::new().with_table(table, schema))
}

pub async fn lower_sql_root_async(
    sql: &str,
    catalog: &SqlCatalog,
    accuracy: AccuracyTarget,
) -> Result<Rc<QueryExpr>, EvalError> {
    lower_sql(sql, catalog, accuracy)
        .await
        .map(Rc::new)
        .map_err(|err| EvalError::Planning(format!("lower {sql:?}: {err}")))
}

pub fn lower_sql_root(
    sql: &str,
    catalog: &SqlCatalog,
    accuracy: AccuracyTarget,
) -> Result<Rc<QueryExpr>, EvalError> {
    runtime().block_on(lower_sql_root_async(sql, catalog, accuracy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_datagen::column::ColumnSpec;
    use aqpbm_datagen::dist::{DataDistribution, UniformParameter};

    fn spec(label: &str, data_type: &str, sql_type: Option<&str>) -> (String, ColumnSpec) {
        (
            label.to_string(),
            ColumnSpec {
                distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: 0.0,
                    upper_bound: 8.0,
                    seed: 1,
                }),
                shift: None,
                cardinality: None,
                special_rule: 0,
                data_type: data_type.to_string(),
                sql_type: sql_type.map(str::to_owned),
                string: None,
            },
        )
    }

    fn description(columns: Vec<(String, ColumnSpec)>) -> TableDescription {
        TableDescription {
            column_num: columns.len() as u32,
            column_label: columns.iter().map(|(label, _)| label.clone()).collect(),
            column_spec: columns.into_iter().map(|(_, spec)| spec).collect(),
            column_connected: Vec::new(),
            row_num: 10,
        }
    }

    #[test]
    fn every_datagen_column_kind_has_one_sql_type() {
        let catalog = catalog_from_spec(
            "metrics",
            &description(vec![
                spec("ts", "i64", Some("timestamp_ms")),
                spec("service", "string", None),
                spec("latency", "f64", None),
                spec("bytes", "i64", None),
                spec("packets", "u64", None),
            ]),
        )
        .expect("every column kind maps");
        let schema = &catalog.tables["metrics"];
        let types: Vec<&DataType> = schema.columns.iter().map(|c| &c.dtype).collect();
        assert_eq!(
            types,
            vec![
                &DataType::Timestamp,
                &DataType::Utf8,
                &DataType::Float64,
                &DataType::Int64,
                &DataType::Int64,
            ]
        );
        assert_eq!(schema.time_index, Some(0));
        assert!(schema.columns.iter().all(|column| !column.nullable));
        assert!(schema.unique_keys.is_empty());
    }

    #[test]
    fn an_i64_column_is_a_number_until_the_spec_says_it_is_an_instant() {
        let plain = catalog_from_spec("t", &description(vec![spec("ts", "i64", None)]))
            .expect("an i64 column maps");
        assert_eq!(plain.tables["t"].columns[0].dtype, DataType::Int64);
        assert_eq!(plain.tables["t"].time_index, None);

        let declared = catalog_from_spec(
            "t",
            &description(vec![spec("ts", "i64", Some("timestamp_ms"))]),
        )
        .expect("a declared timestamp maps");
        assert_eq!(declared.tables["t"].columns[0].dtype, DataType::Timestamp);
        assert_eq!(declared.tables["t"].time_index, Some(0));
    }

    #[test]
    fn a_table_is_named_after_its_spec_file() {
        assert_eq!(
            table_name_from_path(std::path::Path::new("configs/datagen/planeval_sql.yaml"))
                .expect("a stem"),
            "planeval_sql"
        );
    }

    #[test]
    fn the_same_runtime_serves_every_lowering() {
        let catalog = catalog_from_spec(
            "metrics",
            &description(vec![
                spec("latency", "f64", None),
                spec("bytes", "i64", None),
            ]),
        )
        .expect("maps");
        let first = std::ptr::from_ref(runtime());
        lower_sql_root(
            "SELECT SUM(bytes) FROM metrics",
            &catalog,
            AccuracyTarget::Exact,
        )
        .expect("lowers");
        let second = std::ptr::from_ref(runtime());
        assert_eq!(first, second);
    }

    #[test]
    fn a_column_the_query_does_not_name_still_has_to_have_a_type() {
        let err = catalog_from_spec(
            "t",
            &description(vec![spec("v", "f64", Some("timestamp_ms"))]),
        )
        .expect_err("f64 is not an instant");
        assert!(
            matches!(err, EvalError::Planning(ref detail) if detail.contains("timestamp_ms")),
            "{err:?}"
        );
    }
}
