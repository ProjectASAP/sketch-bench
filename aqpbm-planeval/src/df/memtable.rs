use std::sync::Arc;

use aqpbm_datagen::table::GeneratedTable;
use aqpbm_datagen::value::ColumnData;
use datafusion::arrow::array::{
    ArrayRef, Float64Array, Int64Array, StringArray, TimestampMillisecondArray, UInt64Array,
};
use datafusion::arrow::datatypes::{
    DataType as ArrowDataType, Field, Schema as ArrowSchema, SchemaRef,
};
use datafusion::arrow::error::ArrowError;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::error::DataFusionError;
use datafusion::prelude::SessionContext;

use crate::df::refusal::Refusal;
use crate::df::schema::{generated_column_arrow_type, DeclaredSqlType};

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error("the table holds {columns} columns but {declared} SQL types were declared")]
    DeclarationCount { columns: usize, declared: usize },
    #[error(transparent)]
    Arrow(#[from] ArrowError),
    #[error(transparent)]
    DataFusion(#[from] DataFusionError),
}

pub fn rendering_sql_types(table: &GeneratedTable) -> Vec<DeclaredSqlType> {
    vec![DeclaredSqlType::FromRendering; table.data.len()]
}

pub fn generated_arrow_schema(
    table: &GeneratedTable,
    declared: &[DeclaredSqlType],
) -> Result<SchemaRef, IngestError> {
    check_declaration_count(table, declared)?;
    let fields = table
        .data
        .iter()
        .zip(declared)
        .zip(&table.column_title)
        .map(|((column, declared), title)| {
            Ok(Field::new(
                title,
                generated_column_arrow_type(column, *declared)?,
                false,
            ))
        })
        .collect::<Result<Vec<_>, IngestError>>()?;
    Ok(Arc::new(ArrowSchema::new(fields)))
}

pub fn generated_record_batch(
    table: &GeneratedTable,
    declared: &[DeclaredSqlType],
) -> Result<RecordBatch, IngestError> {
    let schema = generated_arrow_schema(table, declared)?;
    let arrays = table
        .data
        .iter()
        .zip(schema.fields())
        .map(|(column, field)| arrow_array(column, field.data_type()))
        .collect::<Result<Vec<_>, IngestError>>()?;
    Ok(RecordBatch::try_new(schema, arrays)?)
}

pub fn register_generated_table(
    context: &SessionContext,
    name: &str,
    table: &GeneratedTable,
    declared: &[DeclaredSqlType],
) -> Result<RecordBatch, IngestError> {
    let batch = generated_record_batch(table, declared)?;
    let provider = MemTable::try_new(batch.schema(), vec![vec![batch.clone()]])?;
    context.register_table(name, Arc::new(provider))?;
    Ok(batch)
}

fn arrow_array(column: &ColumnData, target: &ArrowDataType) -> Result<ArrayRef, IngestError> {
    match (column, target) {
        (ColumnData::Int64(values), ArrowDataType::Int64) => {
            Ok(Arc::new(Int64Array::from(values.clone())))
        }
        (ColumnData::Int64(values), ArrowDataType::Timestamp(_, None)) => {
            Ok(Arc::new(TimestampMillisecondArray::from(values.clone())))
        }
        (ColumnData::Float64(values), ArrowDataType::Float64) => {
            Ok(Arc::new(Float64Array::from(values.clone())))
        }
        (ColumnData::Unsigned64(values), ArrowDataType::UInt64) => {
            Ok(Arc::new(UInt64Array::from(values.clone())))
        }
        (ColumnData::String(values), ArrowDataType::Utf8) => {
            Ok(Arc::new(StringArray::from(values.clone())))
        }
        (column, target) => Err(Refusal::deferred(
            format!("{} column as {target}", column.kind()),
            "column-rendering",
            "the type table gives this rendering a different Arrow type",
        )
        .into()),
    }
}

fn check_declaration_count(
    table: &GeneratedTable,
    declared: &[DeclaredSqlType],
) -> Result<(), IngestError> {
    if table.data.len() != declared.len() {
        return Err(IngestError::DeclarationCount {
            columns: table.data.len(),
            declared: declared.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::df::session::{MemoryPoolSettings, NoSeedBoundFunctions, SeedSession};
    use aqpbm_datagen::table::TableDescription;
    use datafusion::arrow::array::{AsArray, StringArray};
    use datafusion::arrow::datatypes::{Float64Type, Int64Type, UInt64Type};
    use std::path::Path;

    fn spec(name: &str, row_num: u64) -> TableDescription {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("configs/datagen")
            .join(name);
        let mut description = TableDescription::from_path(&path).unwrap();
        description.row_num = row_num;
        description.validate().unwrap();
        description
    }

    fn session() -> SeedSession {
        SeedSession::new(0, MemoryPoolSettings::default(), &NoSeedBoundFunctions).unwrap()
    }

    async fn one_number(session: &SeedSession, sql: &str) -> f64 {
        let batches = session
            .context()
            .sql(sql)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let batch = batches.first().expect("one batch");
        let column = batch.column(0);
        match column.data_type() {
            ArrowDataType::Int64 => column.as_primitive::<Int64Type>().value(0) as f64,
            ArrowDataType::UInt64 => column.as_primitive::<UInt64Type>().value(0) as f64,
            ArrowDataType::Float64 => column.as_primitive::<Float64Type>().value(0),
            other => panic!("{sql} answered with {other}"),
        }
    }

    #[tokio::test]
    async fn count_star_equals_the_spec_row_number() {
        let description = spec("planeval_cluster_metrics.yaml", 20_000);
        let table = description.generate().unwrap();
        let session = session();
        register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
            .unwrap();
        assert_eq!(
            one_number(&session, "SELECT count(*) FROM t").await,
            description.row_num as f64
        );
    }

    #[tokio::test]
    async fn every_column_stays_inside_the_datagen_domain() {
        let description = spec("planeval_cluster_metrics.yaml", 20_000);
        let table = description.generate().unwrap();
        let session = session();
        register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
            .unwrap();

        for (index, title) in table.column_title.iter().enumerate() {
            let column = &description.column_spec[index];
            let shift = column.shift.unwrap_or(0.0);
            let Some(domain) = column.distribution.domain() else {
                continue;
            };
            if column.special_rule != 0 {
                continue;
            }
            if matches!(table.data[index], ColumnData::String(_)) {
                let distinct =
                    one_number(&session, &format!("SELECT count(DISTINCT {title}) FROM t")).await;
                assert!(
                    distinct <= domain.size as f64,
                    "{title} holds {distinct} distinct values over a domain of {}",
                    domain.size
                );
                continue;
            }
            let low = one_number(&session, &format!("SELECT min({title}) FROM t")).await;
            let high = one_number(&session, &format!("SELECT max({title}) FROM t")).await;
            let lower = domain.lower + shift;
            let upper = domain.lower + domain.size as f64 + shift;
            assert!(
                low >= lower && high <= upper,
                "{title} spans [{low}, {high}], outside the datagen domain [{lower}, {upper}]"
            );
        }
    }

    #[tokio::test]
    async fn the_whole_spec_registers_and_reports_its_count_and_bounds() {
        let description = spec("planeval_cluster_metrics.yaml", 200_000);
        let table = description.generate().unwrap();
        let session = session();
        let batch =
            register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
                .unwrap();
        let counted = one_number(&session, "SELECT count(*) FROM t").await;
        println!("spec     configs/datagen/planeval_cluster_metrics.yaml");
        println!(
            "table    {} columns x {} rows, {} bytes of Arrow",
            batch.num_columns(),
            description.row_num,
            batch.get_array_memory_size()
        );
        println!("count(*) {counted} = row_num {}", description.row_num);
        assert_eq!(counted, description.row_num as f64);

        for (index, title) in table.column_title.iter().enumerate() {
            let column = &description.column_spec[index];
            let shift = column.shift.unwrap_or(0.0);
            let domain = column.distribution.domain();
            if matches!(table.data[index], ColumnData::String(_)) {
                let distinct =
                    one_number(&session, &format!("SELECT count(DISTINCT {title}) FROM t")).await;
                println!(
                    "{title:<13} distinct {distinct} over a domain of {}",
                    domain.map_or(0, |d| d.size)
                );
                assert!(distinct <= domain.unwrap().size as f64);
                continue;
            }
            let low = one_number(&session, &format!("SELECT min({title}) FROM t")).await;
            let high = one_number(&session, &format!("SELECT max({title}) FROM t")).await;
            match (domain, column.special_rule) {
                (Some(domain), 0) => {
                    let lower = domain.lower + shift;
                    let upper = domain.lower + domain.size as f64 + shift;
                    println!("{title:<13} [{low}, {high}] inside domain [{lower}, {upper}]");
                    assert!(low >= lower && high <= upper);
                }
                (Some(_), _) => {
                    println!("{title:<13} [{low}, {high}] cumulative from shift {shift}");
                    assert!(low >= shift);
                }
                (None, _) => {
                    println!("{title:<13} [{low}, {high}] unbounded distribution");
                }
            }
        }
    }

    #[tokio::test]
    async fn a_monotonic_column_starts_at_its_shift_and_never_goes_back() {
        let description = spec("planeval_cluster_metrics.yaml", 20_000);
        let table = description.generate().unwrap();
        let session = session();
        register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
            .unwrap();
        let shift = description.column_spec[0].shift.unwrap();
        let low = one_number(&session, "SELECT min(ts) FROM t").await;
        let high = one_number(&session, "SELECT max(ts) FROM t").await;
        assert!(low >= shift, "ts starts at {low}, below its shift {shift}");
        let upper = shift + description.row_num as f64 * 20.0;
        assert!(high <= upper, "ts reaches {high}, past {upper}");
    }

    #[tokio::test]
    async fn the_registered_schema_is_the_one_the_spec_describes() {
        let description = spec("planeval_cluster_metrics.yaml", 1_000);
        let table = description.generate().unwrap();
        let session = session();
        let batch =
            register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
                .unwrap();
        let schema = batch.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(names, description.column_label);
        assert_eq!(batch.schema().field(0).data_type(), &ArrowDataType::Int64);
        assert_eq!(batch.schema().field(1).data_type(), &ArrowDataType::Utf8);
        assert_eq!(batch.schema().field(3).data_type(), &ArrowDataType::Float64);
        assert_eq!(batch.num_rows(), 1_000);
    }

    #[tokio::test]
    async fn a_declared_timestamp_column_registers_as_a_timestamp() {
        let description = spec("planeval_cluster_metrics.yaml", 1_000);
        let table = description.generate().unwrap();
        let mut declared = rendering_sql_types(&table);
        declared[0] = DeclaredSqlType::TimestampMillis;
        let session = session();
        let batch = register_generated_table(session.context(), "t", &table, &declared).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &ArrowDataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None)
        );
        let rows = session
            .context()
            .sql("SELECT date_part('year', ts) AS y FROM t LIMIT 1")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(
            rows[0].column(0).as_primitive::<Float64Type>().value(0),
            2023.0
        );
    }

    #[tokio::test]
    async fn a_u64_column_registers_unsigned() {
        let description = spec("hydra_columns_u64.yaml", 1_000);
        let table = description.generate().unwrap();
        let session = session();
        let batch =
            register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
                .unwrap();
        assert_eq!(batch.schema().field(0).data_type(), &ArrowDataType::Utf8);
        assert_eq!(batch.schema().field(2).data_type(), &ArrowDataType::UInt64);
        assert_eq!(
            one_number(&session, "SELECT count(*) FROM t").await,
            1_000.0
        );
    }

    #[tokio::test]
    async fn a_declaration_for_the_wrong_column_count_is_an_error() {
        let description = spec("planeval_cluster_metrics.yaml", 10);
        let table = description.generate().unwrap();
        let error = generated_record_batch(&table, &[DeclaredSqlType::FromRendering]).unwrap_err();
        assert!(
            matches!(
                error,
                IngestError::DeclarationCount {
                    columns: 6,
                    declared: 1
                }
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn the_group_keys_survive_the_crossing() {
        let description = spec("planeval_cluster_metrics.yaml", 5_000);
        let table = description.generate().unwrap();
        let session = session();
        register_generated_table(session.context(), "t", &table, &rendering_sql_types(&table))
            .unwrap();
        let batches = session
            .context()
            .sql("SELECT cluster, count(*) AS n FROM t GROUP BY cluster ORDER BY cluster")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let total: i64 = batches
            .iter()
            .map(|batch| {
                batch
                    .column(1)
                    .as_primitive::<Int64Type>()
                    .values()
                    .iter()
                    .sum::<i64>()
            })
            .sum();
        assert_eq!(total, 5_000);
        let keys: Vec<String> = batches
            .iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .map(|value| value.unwrap().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(keys.len() <= 8, "cluster holds {} keys", keys.len());
    }
}
