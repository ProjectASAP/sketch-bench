use aqpbm_datagen::table::TableDescription;
use aqpbm_datagen::value::ColumnData;
use asap_types::post_asap::{ExactKind, SummaryFamilyType, SummarySchema};
use asap_types::pre_asap::{Column, DataType, Schema};
use datafusion::arrow::datatypes::{
    DataType as ArrowDataType, Field, IntervalUnit, Schema as ArrowSchema, TimeUnit,
};

use crate::df::refusal::Refusal;

pub const TIMESTAMP_UNIT: TimeUnit = TimeUnit::Millisecond;
pub const INTERVAL_UNIT: IntervalUnit = IntervalUnit::MonthDayNano;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DeclaredSqlType {
    #[default]
    FromRendering,
    TimestampMillis,
}

impl DeclaredSqlType {
    pub const TIMESTAMP_MILLIS: &'static str = "timestamp_ms";

    pub fn parse(spelling: &str) -> Result<Self, Refusal> {
        match spelling {
            Self::TIMESTAMP_MILLIS => Ok(DeclaredSqlType::TimestampMillis),
            other => Err(Refusal::deferred(
                format!("sql_type: {other}"),
                "sql-type-spelling",
                format!(
                    "the only declared SQL type is `{}`; a column without one keeps the type its \
                     datagen rendering produces",
                    Self::TIMESTAMP_MILLIS
                ),
            )),
        }
    }
}

pub fn arrow_type(dtype: &DataType) -> Result<ArrowDataType, Refusal> {
    match dtype {
        DataType::Null => Ok(ArrowDataType::Null),
        DataType::Int64 => Ok(ArrowDataType::Int64),
        DataType::Float64 => Ok(ArrowDataType::Float64),
        DataType::Utf8 => Ok(ArrowDataType::Utf8),
        DataType::Bool => Ok(ArrowDataType::Boolean),
        DataType::Timestamp => Ok(ArrowDataType::Timestamp(TIMESTAMP_UNIT, None)),
        DataType::Date => Ok(ArrowDataType::Date32),
        DataType::Interval => Ok(ArrowDataType::Interval(INTERVAL_UNIT)),
        DataType::List { .. } => Err(Refusal::deferred(
            "DataType::List",
            "nested-column-type",
            "no row of the type table covers a list column",
        )),
        DataType::Struct { .. } => Err(Refusal::deferred(
            "DataType::Struct",
            "nested-column-type",
            "no row of the type table covers a struct column",
        )),
        DataType::Map { .. } => Err(Refusal::deferred(
            "DataType::Map",
            "nested-column-type",
            "no row of the type table covers a map column",
        )),
    }
}

pub fn ir_type(arrow: &ArrowDataType) -> Result<DataType, Refusal> {
    match arrow {
        ArrowDataType::Null => Ok(DataType::Null),
        ArrowDataType::Int8
        | ArrowDataType::Int16
        | ArrowDataType::Int32
        | ArrowDataType::Int64 => Ok(DataType::Int64),
        ArrowDataType::Float32 | ArrowDataType::Float64 => Ok(DataType::Float64),
        ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 => Ok(DataType::Utf8),
        ArrowDataType::Boolean => Ok(DataType::Bool),
        ArrowDataType::Timestamp(_, _) => Ok(DataType::Timestamp),
        ArrowDataType::Date32 | ArrowDataType::Date64 => Ok(DataType::Date),
        ArrowDataType::Interval(_) => Ok(DataType::Interval),
        ArrowDataType::Binary | ArrowDataType::LargeBinary => Err(Refusal::no_constructor(
            "ArrowDataType::Binary",
            "a Binary column carries summary state, which the pre-ASAP IR has no type for",
        )),
        other => Err(Refusal::deferred(
            format!("ArrowDataType::{other}"),
            "arrow-type-outside-the-table",
            "no row of the type table maps this Arrow type back to the IR",
        )),
    }
}

pub fn arrow_field(column: &Column) -> Result<Field, Refusal> {
    Ok(Field::new(
        &column.name,
        arrow_type(&column.dtype)?,
        column.nullable,
    ))
}

pub fn arrow_schema(schema: &Schema) -> Result<ArrowSchema, Refusal> {
    let fields = schema
        .columns
        .iter()
        .map(arrow_field)
        .collect::<Result<Vec<_>, Refusal>>()?;
    Ok(ArrowSchema::new(fields))
}

pub fn ir_schema(schema: &ArrowSchema) -> Result<Schema, Refusal> {
    let columns = schema
        .fields()
        .iter()
        .map(|field| {
            Ok(Column::new(
                field.name(),
                ir_type(field.data_type())?,
                field.is_nullable(),
            ))
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    Ok(Schema::new(columns))
}

pub fn summary_state_arrow_type(
    family: &SummaryFamilyType,
    input: &ArrowDataType,
) -> Result<ArrowDataType, Refusal> {
    match family {
        SummaryFamilyType::Plain(_) => Err(Refusal::no_constructor(
            "SummaryFamilyType::Plain",
            "a plain column is a value, not a summary state, and no summary constructor builds \
             one",
        )),
        SummaryFamilyType::Sketch(_, _) => Ok(ArrowDataType::Binary),
        SummaryFamilyType::ExactAggregate(kind, _) => match kind {
            ExactKind::Sum => arrow_type(&ir_type(input)?),
            ExactKind::Min | ExactKind::Max => {
                ir_type(input)?;
                Ok(input.clone())
            }
            ExactKind::Count => Ok(ArrowDataType::Int64),
            ExactKind::Increase | ExactKind::Rate | ExactKind::IRate => Err(Refusal::deferred(
                format!("SummaryFamilyType::ExactAggregate({kind:?})"),
                "order-sensitive",
                "a counter-reset-aware accumulator reads its input in timestamp order",
            )),
        },
        SummaryFamilyType::Sample(_, _) => Err(Refusal::deferred(
            "SummaryFamilyType::Sample",
            "family-outside-the-table",
            "the type table gives no state encoding for a retained row subset",
        )),
        SummaryFamilyType::Wavelet(_, _) => Err(Refusal::deferred(
            "SummaryFamilyType::Wavelet",
            "family-outside-the-table",
            "the type table gives no state encoding for a coefficient vector",
        )),
        SummaryFamilyType::StatModel(_, _) => Err(Refusal::deferred(
            "SummaryFamilyType::StatModel",
            "family-outside-the-table",
            "the type table gives no state encoding for a fitted model",
        )),
    }
}

pub fn summary_field_arrow_type(
    family: &SummaryFamilyType,
    produced: &ArrowDataType,
) -> Result<ArrowDataType, Refusal> {
    match family {
        SummaryFamilyType::Plain(dtype) => arrow_type(dtype),
        other => summary_state_arrow_type(other, produced),
    }
}

pub fn summary_schema_fields(
    schema: &SummarySchema,
    produced: &ArrowSchema,
) -> Result<Vec<Field>, Refusal> {
    if schema.fields.len() != produced.fields().len() {
        return Err(Refusal::no_constructor(
            "SummarySchema",
            format!(
                "the edge declares {} columns and the query it cuts produces {}",
                schema.fields.len(),
                produced.fields().len()
            ),
        ));
    }
    schema
        .fields
        .iter()
        .zip(produced.fields())
        .map(|(declared, made)| {
            if declared.name != *made.name() {
                return Err(Refusal::no_constructor(
                    "SummarySchema",
                    format!(
                        "the edge names this column {} and the query it cuts names it {}",
                        declared.name,
                        made.name()
                    ),
                ));
            }
            Ok(Field::new(
                &declared.name,
                summary_field_arrow_type(&declared.dtype, made.data_type())?,
                declared.nullable || made.is_nullable(),
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredColumn {
    pub sql_type: DeclaredSqlType,
    pub nullable: bool,
}

pub fn declared_columns(description: &TableDescription) -> Result<Vec<DeclaredColumn>, Refusal> {
    description
        .column_spec
        .iter()
        .map(|spec| {
            Ok(DeclaredColumn {
                sql_type: match &spec.sql_type {
                    Some(spelling) => DeclaredSqlType::parse(spelling)?,
                    None => DeclaredSqlType::FromRendering,
                },
                nullable: false,
            })
        })
        .collect()
}

pub fn column_rendering(data_type: &str) -> Result<ColumnData, Refusal> {
    match data_type {
        "i64" => Ok(ColumnData::Int64(Vec::new())),
        "f64" => Ok(ColumnData::Float64(Vec::new())),
        "u64" => Ok(ColumnData::Unsigned64(Vec::new())),
        "string" => Ok(ColumnData::String(Vec::new())),
        other => Err(Refusal::deferred(
            format!("data_type: {other}"),
            "datagen-rendering",
            "this spec spelling names no column rendering datagen produces",
        )),
    }
}

pub fn generated_column_arrow_type(
    column: &ColumnData,
    declared: DeclaredSqlType,
) -> Result<ArrowDataType, Refusal> {
    match (column, declared) {
        (ColumnData::Int64(_), DeclaredSqlType::TimestampMillis) => {
            Ok(ArrowDataType::Timestamp(TIMESTAMP_UNIT, None))
        }
        (other, DeclaredSqlType::TimestampMillis) => Err(Refusal::deferred(
            format!(
                "sql_type: {} on a {} column",
                DeclaredSqlType::TIMESTAMP_MILLIS,
                other.kind()
            ),
            "sql-type-declaration",
            "only an i64 column renders milliseconds since the epoch",
        )),
        (ColumnData::Int64(_), DeclaredSqlType::FromRendering) => Ok(ArrowDataType::Int64),
        (ColumnData::Float64(_), DeclaredSqlType::FromRendering) => Ok(ArrowDataType::Float64),
        (ColumnData::Unsigned64(_), DeclaredSqlType::FromRendering) => Ok(ArrowDataType::UInt64),
        (ColumnData::String(_), DeclaredSqlType::FromRendering) => Ok(ArrowDataType::Utf8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_types::post_asap::{
        ExactParams, GroupingStrategy, SamplingKind, SamplingParams, SketchAlgorithm, SketchKind,
        SketchParams, SummaryField,
    };

    fn kll() -> SummaryFamilyType {
        SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
            GroupingStrategy::PerSubpopulationInstance,
        )
    }

    #[test]
    fn scalar_rows_of_the_type_table() {
        assert_eq!(arrow_type(&DataType::Null).unwrap(), ArrowDataType::Null);
        assert_eq!(arrow_type(&DataType::Int64).unwrap(), ArrowDataType::Int64);
        assert_eq!(
            arrow_type(&DataType::Float64).unwrap(),
            ArrowDataType::Float64
        );
        assert_eq!(arrow_type(&DataType::Utf8).unwrap(), ArrowDataType::Utf8);
        assert_eq!(arrow_type(&DataType::Bool).unwrap(), ArrowDataType::Boolean);
        assert_eq!(
            arrow_type(&DataType::Timestamp).unwrap(),
            ArrowDataType::Timestamp(TimeUnit::Millisecond, None)
        );
        assert_eq!(arrow_type(&DataType::Date).unwrap(), ArrowDataType::Date32);
        assert_eq!(
            arrow_type(&DataType::Interval).unwrap(),
            ArrowDataType::Interval(IntervalUnit::MonthDayNano)
        );
    }

    #[test]
    fn nested_types_are_refused_by_name() {
        let list = DataType::List {
            element: Box::new(Column::new("item", DataType::Int64, true)),
        };
        let refused = arrow_type(&list).unwrap_err();
        assert_eq!(refused.variant, "DataType::List");
        assert_eq!(refused.tag(), "deferred");

        let map = DataType::Map {
            key: Box::new(DataType::Utf8),
            value: Box::new(DataType::Int64),
            value_nullable: true,
        };
        assert_eq!(arrow_type(&map).unwrap_err().variant, "DataType::Map");

        let structure = DataType::Struct {
            fields: vec![Column::new("a", DataType::Int64, false)],
        };
        assert_eq!(
            arrow_type(&structure).unwrap_err().variant,
            "DataType::Struct"
        );
    }

    #[test]
    fn every_scalar_type_survives_a_round_trip() {
        for dtype in [
            DataType::Null,
            DataType::Int64,
            DataType::Float64,
            DataType::Utf8,
            DataType::Bool,
            DataType::Timestamp,
            DataType::Date,
            DataType::Interval,
        ] {
            let arrow = arrow_type(&dtype).unwrap();
            assert_eq!(ir_type(&arrow).unwrap(), dtype, "round trip of {dtype:?}");
        }
    }

    #[test]
    fn narrower_arrow_widths_widen_to_the_ir_types() {
        assert_eq!(ir_type(&ArrowDataType::Int32).unwrap(), DataType::Int64);
        assert_eq!(ir_type(&ArrowDataType::Float32).unwrap(), DataType::Float64);
        assert_eq!(ir_type(&ArrowDataType::LargeUtf8).unwrap(), DataType::Utf8);
        assert_eq!(ir_type(&ArrowDataType::Date64).unwrap(), DataType::Date);
    }

    #[test]
    fn a_state_column_has_no_ir_type() {
        let refused = ir_type(&ArrowDataType::Binary).unwrap_err();
        assert_eq!(refused.tag(), "no_constructor");
    }

    #[test]
    fn sketch_state_is_binary_and_exact_state_follows_its_input() {
        assert_eq!(
            summary_state_arrow_type(&kll(), &ArrowDataType::Float64).unwrap(),
            ArrowDataType::Binary
        );
        assert_eq!(
            summary_state_arrow_type(
                &SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum),
                &ArrowDataType::Int64
            )
            .unwrap(),
            ArrowDataType::Int64
        );
        assert_eq!(
            summary_state_arrow_type(
                &SummaryFamilyType::ExactAggregate(ExactKind::Max, ExactParams::Max),
                &ArrowDataType::Float64
            )
            .unwrap(),
            ArrowDataType::Float64
        );
        assert_eq!(
            summary_state_arrow_type(
                &SummaryFamilyType::ExactAggregate(ExactKind::Count, ExactParams::Count),
                &ArrowDataType::Utf8
            )
            .unwrap(),
            ArrowDataType::Int64
        );
    }

    #[test]
    fn a_plain_family_names_no_summary_state_but_types_a_schema_field() {
        let refused = summary_state_arrow_type(
            &SummaryFamilyType::Plain(DataType::Utf8),
            &ArrowDataType::Utf8,
        )
        .unwrap_err();
        assert_eq!(refused.variant, "SummaryFamilyType::Plain");
        assert_eq!(refused.tag(), "no_constructor");

        assert_eq!(
            summary_field_arrow_type(
                &SummaryFamilyType::Plain(DataType::Utf8),
                &ArrowDataType::Utf8
            )
            .unwrap(),
            ArrowDataType::Utf8
        );
        assert_eq!(
            summary_field_arrow_type(&kll(), &ArrowDataType::Float64).unwrap(),
            ArrowDataType::Binary
        );
    }

    #[test]
    fn a_state_table_schema_pairs_the_edge_with_what_the_cut_query_produces() {
        let schema = SummarySchema {
            fields: vec![
                SummaryField {
                    name: "service".into(),
                    dtype: SummaryFamilyType::Plain(DataType::Utf8),
                    nullable: false,
                },
                SummaryField {
                    name: "latency".into(),
                    dtype: kll(),
                    nullable: false,
                },
            ],
            time_index: None,
        };
        let produced = ArrowSchema::new(vec![
            Field::new("service", ArrowDataType::Utf8, false),
            Field::new("latency", ArrowDataType::Binary, true),
        ]);
        let fields = summary_schema_fields(&schema, &produced).unwrap();
        assert_eq!(fields[0].data_type(), &ArrowDataType::Utf8);
        assert_eq!(fields[1].data_type(), &ArrowDataType::Binary);
        assert!(fields[1].is_nullable());

        let renamed = ArrowSchema::new(vec![
            Field::new("cluster", ArrowDataType::Utf8, false),
            Field::new("latency", ArrowDataType::Binary, true),
        ]);
        assert_eq!(
            summary_schema_fields(&schema, &renamed).unwrap_err().tag(),
            "no_constructor"
        );
    }

    #[test]
    fn the_families_outside_the_table_are_refused() {
        let refused = summary_state_arrow_type(
            &SummaryFamilyType::Sample(
                SamplingKind::Reservoir,
                SamplingParams::Reservoir { size: 8 },
            ),
            &ArrowDataType::Float64,
        )
        .unwrap_err();
        assert_eq!(refused.variant, "SummaryFamilyType::Sample");

        let refused = summary_state_arrow_type(
            &SummaryFamilyType::ExactAggregate(ExactKind::Rate, ExactParams::Rate),
            &ArrowDataType::Float64,
        )
        .unwrap_err();
        assert_eq!(
            refused.reason,
            crate::df::refusal::RefusalReason::Deferred {
                issue: "order-sensitive".into()
            }
        );
    }

    #[test]
    fn every_rendering_is_named_by_the_spelling_that_selects_it() {
        for column in [
            ColumnData::Int64(Vec::new()),
            ColumnData::Float64(Vec::new()),
            ColumnData::Unsigned64(Vec::new()),
            ColumnData::String(Vec::new()),
        ] {
            assert_eq!(
                column_rendering(column.kind()).expect("the kind names a rendering"),
                column,
                "{}",
                column.kind()
            );
        }
        assert_eq!(
            column_rendering("i128").unwrap_err().tag(),
            "deferred",
            "a spelling datagen does not render is refused, not guessed"
        );
    }

    #[test]
    fn a_shifted_i64_column_is_an_integer_until_the_spec_says_otherwise() {
        let column = ColumnData::Int64(vec![1_700_000_000_000, 1_700_000_000_001]);
        assert_eq!(
            generated_column_arrow_type(&column, DeclaredSqlType::FromRendering).unwrap(),
            ArrowDataType::Int64
        );
        assert_eq!(
            generated_column_arrow_type(&column, DeclaredSqlType::TimestampMillis).unwrap(),
            ArrowDataType::Timestamp(TimeUnit::Millisecond, None)
        );
        assert_eq!(
            DeclaredSqlType::parse("timestamp_ms").unwrap(),
            DeclaredSqlType::TimestampMillis
        );
        assert_eq!(
            DeclaredSqlType::parse("timestamp_us").unwrap_err().tag(),
            "deferred"
        );
    }

    #[test]
    fn each_rendering_has_an_arrow_type() {
        let renderings = [
            (ColumnData::Int64(Vec::new()), ArrowDataType::Int64),
            (ColumnData::Float64(Vec::new()), ArrowDataType::Float64),
            (ColumnData::Unsigned64(Vec::new()), ArrowDataType::UInt64),
            (ColumnData::String(Vec::new()), ArrowDataType::Utf8),
        ];
        for (column, expected) in renderings {
            assert_eq!(
                generated_column_arrow_type(&column, DeclaredSqlType::FromRendering).unwrap(),
                expected
            );
        }
        let refused = generated_column_arrow_type(
            &ColumnData::Float64(Vec::new()),
            DeclaredSqlType::TimestampMillis,
        )
        .unwrap_err();
        assert_eq!(refused.tag(), "deferred");
    }

    #[test]
    fn a_schema_crosses_in_both_directions() {
        let schema = Schema::new(vec![
            Column::new("ts", DataType::Timestamp, false),
            Column::new("cluster", DataType::Utf8, true),
            Column::new("cpu_cores", DataType::Float64, false),
        ]);
        let arrow = arrow_schema(&schema).unwrap();
        assert_eq!(arrow.fields().len(), 3);
        assert_eq!(arrow.field(1).name(), "cluster");
        assert!(arrow.field(1).is_nullable());
        assert_eq!(ir_schema(&arrow).unwrap().columns, schema.columns);
    }
}
