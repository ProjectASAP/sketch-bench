use std::rc::Rc;
use std::sync::Arc;

use aqpbm_datagen::table::GeneratedTable;
use aqpbm_datagen::value::ColumnData;
use aqpbm_planeval::df::pre_asap::TableSources;
use aqpbm_planeval::df::pre_asap_arm::answer_with_tables;
use aqpbm_planeval::df::session::{MemoryPoolSettings, NoSeedBoundFunctions, SeedSession};
use aqpbm_planeval::exact::run_tree;
use aqpbm_planeval::exact::Data;
use aqpbm_planeval::plan::TimeRangeOrigin;
use aqpbm_planeval::run::RowsFrom;
use aqpbm_planeval::Value;
use asap_types::pre_asap::agg_intent::AggIntent;
use asap_types::pre_asap::expr_ir::{CompareOpKind, ScalarValue};
use asap_types::pre_asap::query_expr::{Predicate, Reduction, Source};
use asap_types::pre_asap::schema::{Column, DataType, Schema};
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;
use datafusion::arrow::array::{ArrayRef, Float64Array, Int64Array};
use datafusion::arrow::datatypes::{DataType as ArrowDataType, Field, Schema as ArrowSchema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;

const TABLE: &str = "t";

const VALUE: usize = 0;

const WHOLE: usize = 1;

const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

fn ir_schema(nullable: bool) -> Schema {
    Schema::new(vec![
        Column::new("v", DataType::Float64, nullable),
        Column::new("n", DataType::Int64, nullable),
    ])
}

fn arrow_schema(nullable: bool) -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new("v", ArrowDataType::Float64, nullable),
        Field::new("n", ArrowDataType::Int64, nullable),
    ])
}

fn generated(values: &[f64], wholes: &[i64]) -> Rc<GeneratedTable> {
    Rc::new(GeneratedTable {
        column_num: 2,
        column_title: vec!["v".to_owned(), "n".to_owned()],
        data: vec![
            ColumnData::Float64(values.to_vec()),
            ColumnData::Int64(wholes.to_vec()),
        ],
        row_num: values.len() as u64,
    })
}

fn batch(values: Vec<Option<f64>>, wholes: Vec<Option<i64>>, nullable: bool) -> RecordBatch {
    let schema = Arc::new(arrow_schema(nullable));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from(values)),
        Arc::new(Int64Array::from(wholes)),
    ];
    RecordBatch::try_new(schema, columns).expect("the columns are the same length")
}

fn session_over(batch: RecordBatch) -> (SeedSession, TableSources) {
    let session = SeedSession::new(0, MemoryPoolSettings::default(), &NoSeedBoundFunctions)
        .expect("a session needs no OS resources");
    let table = MemTable::try_new(batch.schema(), vec![vec![batch]]).expect("a one-batch table");
    session
        .context()
        .register_table(TABLE, Arc::new(table))
        .expect("the name is free");
    let tables = futures_of(session.context());
    (session, tables)
}

fn futures_of(context: &SessionContext) -> TableSources {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(TableSources::of_context(context))
    })
    .expect("the registered table is reachable")
}

fn scan(nullable: bool, predicates: Vec<Predicate>) -> Rc<QueryExpr> {
    Rc::new(QueryExpr::Scan {
        source: Source::Table {
            table_ref: TABLE.to_owned(),
        },
        predicates,
        schema: ir_schema(nullable),
    })
}

fn global(
    measure: AggIntent,
    name: &str,
    nullable: bool,
    predicates: Vec<Predicate>,
) -> Rc<QueryExpr> {
    Rc::new(QueryExpr::Aggregate {
        reduction: Reduction::by(Vec::new()),
        measures: vec![measure],
        output_names: vec![name.to_owned()],
        having: None,
        child: scan(nullable, predicates),
    })
}

fn impossible() -> Predicate {
    Predicate(Rc::new(QueryExpr::Compare {
        left: Rc::new(QueryExpr::Column(VALUE)),
        op: CompareOpKind::Lt,
        right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(f64::NEG_INFINITY))),
    }))
}

async fn arm_value(
    tree: &QueryExpr,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<Option<Value>, String> {
    let answered = answer_with_tables(tree, session, tables)
        .await
        .map_err(|error| error.to_string())?;
    let mut held = Vec::new();
    for batch in &answered.batches {
        for row in 0..batch.num_rows() {
            let column = batch.column(0);
            held.push(if column.is_null(row) {
                Value::Null
            } else if let Some(floats) = column.as_any().downcast_ref::<Float64Array>() {
                Value::Float(floats.value(row))
            } else if let Some(wholes) = column.as_any().downcast_ref::<Int64Array>() {
                Value::Int(wholes.value(row))
            } else {
                return Err(format!("no ver 1 value matches {}", column.data_type()));
            });
        }
    }
    Ok(held.into_iter().next())
}

fn interpreted_value(
    tree: Rc<QueryExpr>,
    table: &Rc<GeneratedTable>,
) -> Result<Option<Value>, String> {
    let run = run_tree(
        tree,
        &RowsFrom::Generated(Rc::clone(table)),
        TimeRangeOrigin::Unknown,
    )
    .map_err(|e| e.to_string())?;
    Ok(match run.answer {
        Data::Rows(rows) => rows.first().and_then(|row| row.0.first().cloned()),
        Data::Scalar(value) => Some(Value::Float(value)),
    })
}

fn same(left: &Option<Value>, right: &Option<Value>) -> bool {
    match (left, right) {
        (Some(Value::Float(left)), Some(Value::Float(right))) => left.total_cmp(right).is_eq(),
        (Some(Value::Int(left)), Some(Value::Float(right)))
        | (Some(Value::Float(right)), Some(Value::Int(left))) => {
            (*left as f64).total_cmp(right).is_eq()
        }
        (left, right) => left == right,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nan_in_the_reduced_column_reads_the_same_on_both_arms() {
    let values = [1.0_f64, f64::NAN, 3.0, 2.0];
    let wholes = [1_i64, 2, 3, 4];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    for (name, measure) in [
        ("sum", AggIntent::Sum { col: Some(VALUE) }),
        ("min", AggIntent::Min { col: Some(VALUE) }),
        ("max", AggIntent::Max { col: Some(VALUE) }),
        ("avg", AggIntent::Avg { col: Some(VALUE) }),
        (
            "quantile",
            AggIntent::Quantile {
                col: Some(VALUE),
                q: 0.5,
                accuracy: ACCURACY,
            },
        ),
    ] {
        let tree = global(measure, name, false, Vec::new());
        let arm = arm_value(&tree, &session, &tables).await;
        let interpreted = interpreted_value(Rc::clone(&tree), &table);
        assert!(
            arm.as_ref()
                .ok()
                .zip(interpreted.as_ref().ok())
                .is_some_and(|(left, right)| same(left, right)),
            "{name}: arm A {arm:?} and ver 1 {interpreted:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_exact_quantile_takes_the_floor_of_q_times_the_row_count_clamped_to_the_last_position()
{
    let values = [4.0_f64, 1.0, 3.0, 2.0];
    let wholes = [1_i64, 2, 3, 4];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    for (q, wanted) in [
        (0.0, 1.0),
        (0.2, 1.0),
        (0.25, 2.0),
        (0.49, 2.0),
        (0.5, 3.0),
        (0.74, 3.0),
        (0.75, 4.0),
        (1.0, 4.0),
    ] {
        let tree = global(
            AggIntent::Quantile {
                col: Some(VALUE),
                q,
                accuracy: ACCURACY,
            },
            "q",
            false,
            Vec::new(),
        );
        let arm = arm_value(&tree, &session, &tables).await.expect("answers");
        let interpreted = interpreted_value(Rc::clone(&tree), &table).expect("answers");
        assert_eq!(arm, Some(Value::Float(wanted)), "q = {q} on arm A");
        assert_eq!(interpreted, Some(Value::Float(wanted)), "q = {q} on ver 1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tied_column_answers_the_tied_value_at_every_position_it_occupies() {
    let values = [5.0_f64, 5.0, 5.0, 9.0];
    let wholes = [1_i64, 2, 3, 4];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    for (q, wanted) in [(0.0, 5.0), (0.5, 5.0), (0.74, 5.0), (0.75, 9.0), (1.0, 9.0)] {
        let tree = global(
            AggIntent::Quantile {
                col: Some(VALUE),
                q,
                accuracy: ACCURACY,
            },
            "q",
            false,
            Vec::new(),
        );
        let arm = arm_value(&tree, &session, &tables).await.expect("answers");
        let interpreted = interpreted_value(Rc::clone(&tree), &table).expect("answers");
        assert_eq!(arm, Some(Value::Float(wanted)), "q = {q} on arm A");
        assert_eq!(interpreted, Some(Value::Float(wanted)), "q = {q} on ver 1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_group_reads_the_same_on_both_arms() {
    let values = [1.0_f64, 2.0];
    let wholes = [1_i64, 2];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    for (name, measure) in [
        ("sum", AggIntent::Sum { col: Some(VALUE) }),
        ("min", AggIntent::Min { col: Some(VALUE) }),
        ("avg", AggIntent::Avg { col: Some(VALUE) }),
        (
            "quantile",
            AggIntent::Quantile {
                col: Some(VALUE),
                q: 0.5,
                accuracy: ACCURACY,
            },
        ),
        ("count", AggIntent::Count { accuracy: ACCURACY }),
        (
            "cardinality",
            AggIntent::Cardinality {
                col: Some(VALUE),
                accuracy: ACCURACY,
            },
        ),
    ] {
        let tree = global(measure, name, false, vec![impossible()]);
        let arm = arm_value(&tree, &session, &tables).await;
        let interpreted = interpreted_value(Rc::clone(&tree), &table);
        assert!(
            arm.as_ref()
                .ok()
                .zip(interpreted.as_ref().ok())
                .is_some_and(|(left, right)| same(left, right)),
            "{name}: arm A {arm:?} and ver 1 {interpreted:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_integer_sum_past_an_i64_wraps_on_arm_a_and_is_refused_by_the_interpreted_arm() {
    let values = [1.0_f64, 2.0];
    let wholes = [i64::MAX, 1];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    let tree = global(
        AggIntent::Sum { col: Some(WHOLE) },
        "total",
        false,
        Vec::new(),
    );

    let interpreted = interpreted_value(Rc::clone(&tree), &table)
        .expect_err("ver 1 folds through an i128 and reports the overflow");
    assert!(interpreted.contains("past an i64"), "{interpreted}");

    let arm = arm_value(&tree, &session, &tables)
        .await
        .expect("DataFusion answers");
    assert_eq!(
        arm,
        Some(Value::Int(i64::MIN)),
        "DataFusion 43 wraps an Int64 sum rather than reporting it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_null_in_the_reduced_column_is_skipped_and_still_counted_by_count_star() {
    let (session, tables) = session_over(batch(
        vec![Some(1.0), None, Some(3.0)],
        vec![Some(1), Some(2), Some(3)],
        true,
    ));

    let sum = global(
        AggIntent::Sum { col: Some(VALUE) },
        "total",
        true,
        Vec::new(),
    );
    assert_eq!(
        arm_value(&sum, &session, &tables).await.unwrap(),
        Some(Value::Float(4.0))
    );

    let count = global(
        AggIntent::Count { accuracy: ACCURACY },
        "rows",
        true,
        Vec::new(),
    );
    assert_eq!(
        arm_value(&count, &session, &tables).await.unwrap(),
        Some(Value::Int(3))
    );

    let quantile = global(
        AggIntent::Quantile {
            col: Some(VALUE),
            q: 0.5,
            accuracy: ACCURACY,
        },
        "median",
        true,
        Vec::new(),
    );
    assert_eq!(
        arm_value(&quantile, &session, &tables).await.unwrap(),
        Some(Value::Float(3.0))
    );

    let cardinality = global(
        AggIntent::Cardinality {
            col: Some(VALUE),
            accuracy: ACCURACY,
        },
        "distinct",
        true,
        Vec::new(),
    );
    assert_eq!(
        arm_value(&cardinality, &session, &tables).await.unwrap(),
        Some(Value::Int(2))
    );
}
