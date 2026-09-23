use std::rc::Rc;
use std::sync::Arc;

use aqpbm_datagen::table::GeneratedTable;
use aqpbm_datagen::value::ColumnData;
use aqpbm_planeval::df::pre_asap::TableSources;
use aqpbm_planeval::df::pre_asap_arm::answer_with_tables;
use aqpbm_planeval::df::session::{MemoryPoolSettings, NoSeedBoundFunctions, SeedSession};
use aqpbm_planeval::exact::run_tree;
use aqpbm_planeval::exact::Data;
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
    let run = run_tree(tree, &RowsFrom::Generated(Rc::clone(table))).map_err(|e| e.to_string())?;
    Ok(match run.answer {
        Data::Rows(rows) => rows.first().and_then(|row| row.0.first().cloned()),
        Data::Scalar(value) => Some(Value::Float(value)),
    })
}

fn same(left: &Option<Value>, right: &Option<Value>) -> bool {
    match (left, right) {
        (Some(Value::Float(left)), Some(Value::Float(right))) => left.total_cmp(right).is_eq(),
        (Some(Value::Int(left)), Some(Value::Float(right)))
        | (Some(Value::Float(right)), Some(Value::Int(left))) => whole_equals_float(*left, *right),
        (left, right) => left == right,
    }
}

fn whole_equals_float(whole: i64, float: f64) -> bool {
    if !float.is_finite() || float.fract() != 0.0 {
        return false;
    }
    let exact = float as i128;
    exact as f64 == float && exact == i128::from(whole)
}

fn close(answered: f64, wanted: f64) -> bool {
    (answered - wanted).abs() <= 1e-12 * wanted.abs().max(1.0)
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

    let mut measures: Vec<(String, AggIntent)> = vec![
        ("sum".to_owned(), AggIntent::Sum { col: Some(VALUE) }),
        ("min".to_owned(), AggIntent::Min { col: Some(VALUE) }),
        ("max".to_owned(), AggIntent::Max { col: Some(VALUE) }),
        ("avg".to_owned(), AggIntent::Avg { col: Some(VALUE) }),
        (
            "cardinality".to_owned(),
            AggIntent::Cardinality {
                col: Some(VALUE),
                accuracy: ACCURACY,
            },
        ),
    ];
    for q in [0.0_f64, 0.25, 0.5, 0.74, 0.75, 1.0] {
        measures.push((
            format!("quantile at q = {q}"),
            AggIntent::Quantile {
                col: Some(VALUE),
                q,
                accuracy: ACCURACY,
            },
        ));
    }

    for (name, measure) in measures {
        let tree = global(measure, &name, false, Vec::new());
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

    let last = global(
        AggIntent::Quantile {
            col: Some(VALUE),
            q: 1.0,
            accuracy: ACCURACY,
        },
        "top",
        false,
        Vec::new(),
    );
    let Some(Value::Float(top)) = arm_value(&last, &session, &tables).await.expect("answers")
    else {
        panic!("a quantile over a float column answers a float");
    };
    assert!(
        top.is_nan(),
        "both arms sort the NaN to the end of the column, so q = 1 reads it: {top}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_spread_measures_answer_what_the_same_rows_give_by_hand() {
    let values = [1.0_f64, 2.0, 4.0, 8.0];
    let wholes = [1_i64, 2, 3, 4];
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    let rows = values.len() as f64;
    let mean = values.iter().sum::<f64>() / rows;
    let squares: f64 = values.iter().map(|value| (value - mean).powi(2)).sum();
    let sample = squares / (rows - 1.0);
    let population = squares / rows;

    for (name, measure, wanted) in [
        (
            "variance, sample",
            AggIntent::Variance {
                col: Some(VALUE),
                population: false,
            },
            sample,
        ),
        (
            "variance, population",
            AggIntent::Variance {
                col: Some(VALUE),
                population: true,
            },
            population,
        ),
        (
            "stddev, sample",
            AggIntent::StdDev {
                col: Some(VALUE),
                population: false,
            },
            sample.sqrt(),
        ),
        (
            "stddev, population",
            AggIntent::StdDev {
                col: Some(VALUE),
                population: true,
            },
            population.sqrt(),
        ),
    ] {
        let tree = global(measure, "spread", false, Vec::new());
        let Some(Value::Float(answered)) = arm_value(&tree, &session, &tables)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"))
        else {
            panic!("{name} answers a float");
        };
        assert!(
            close(answered, wanted),
            "{name}: arm A answered {answered} and these rows give {wanted}"
        );
    }

    assert!(
        !close(sample, population),
        "the two estimators differ on these rows ({sample} and {population}), so a swapped \
         population flag cannot read as agreement"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pearson_correlation_answers_what_the_same_two_columns_give_by_hand() {
    let values = [1.0_f64, 2.0, 4.0, 8.0];
    let wholes = [1_i64, 2, 3, 4];
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    let left: Vec<f64> = values.to_vec();
    let right: Vec<f64> = wholes.iter().map(|whole| *whole as f64).collect();
    let rows = left.len() as f64;
    let mean_left = left.iter().sum::<f64>() / rows;
    let mean_right = right.iter().sum::<f64>() / rows;
    let covariance: f64 = left
        .iter()
        .zip(&right)
        .map(|(x, y)| (x - mean_left) * (y - mean_right))
        .sum();
    let spread_left: f64 = left.iter().map(|x| (x - mean_left).powi(2)).sum();
    let spread_right: f64 = right.iter().map(|y| (y - mean_right).powi(2)).sum();
    let wanted = covariance / (spread_left.sqrt() * spread_right.sqrt());

    let tree = global(
        AggIntent::PearsonCorr {
            left: VALUE,
            right: WHOLE,
        },
        "r",
        false,
        Vec::new(),
    );
    let Some(Value::Float(answered)) = arm_value(&tree, &session, &tables)
        .await
        .expect("a correlation answers")
    else {
        panic!("a correlation answers a float");
    };
    assert!(
        close(answered, wanted),
        "arm A answered {answered} and these two columns give {wanted}"
    );
    assert!(
        (0.0..1.0).contains(&wanted),
        "the two columns rise together without being the same column: {wanted}"
    );

    let itself = global(
        AggIntent::PearsonCorr {
            left: VALUE,
            right: VALUE,
        },
        "r",
        false,
        Vec::new(),
    );
    let Some(Value::Float(perfect)) = arm_value(&itself, &session, &tables)
        .await
        .expect("a correlation answers")
    else {
        panic!("a correlation answers a float");
    };
    assert!(
        close(perfect, 1.0),
        "a column against itself correlates at 1, not {perfect}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_integer_quantile_past_two_to_the_fifty_third_parts_from_the_float_ver_one_answers() {
    let big = (1_i64 << 53) + 1;
    let values = [1.0_f64, 2.0, 3.0, 4.0];
    let wholes = [big, 1, 2, 3];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    let tree = global(
        AggIntent::Quantile {
            col: Some(WHOLE),
            q: 1.0,
            accuracy: ACCURACY,
        },
        "top",
        false,
        Vec::new(),
    );
    let arm = arm_value(&tree, &session, &tables).await.expect("answers");
    let interpreted = interpreted_value(Rc::clone(&tree), &table).expect("answers");
    assert_eq!(arm, Some(Value::Int(big)));
    assert_eq!(interpreted, Some(Value::Float(9_007_199_254_740_992.0)));
    assert!(
        (big as f64).total_cmp(&9_007_199_254_740_992.0).is_eq(),
        "a comparison that casts the integer through an f64 cannot tell these two apart"
    );
    assert!(
        !same(&arm, &interpreted),
        "above 2^53 the two arms answer different numbers and the comparison says so"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_integer_quantile_below_two_to_the_fifty_third_still_matches_the_interpreted_arm() {
    let big = (1_i64 << 53) - 1;
    let values = [1.0_f64, 2.0, 3.0, 4.0];
    let wholes = [big, 1, 2, 3];
    let table = generated(&values, &wholes);
    let (session, tables) = session_over(batch(
        values.iter().copied().map(Some).collect(),
        wholes.iter().copied().map(Some).collect(),
        false,
    ));

    let tree = global(
        AggIntent::Quantile {
            col: Some(WHOLE),
            q: 1.0,
            accuracy: ACCURACY,
        },
        "top",
        false,
        Vec::new(),
    );
    let arm = arm_value(&tree, &session, &tables).await.expect("answers");
    let interpreted = interpreted_value(Rc::clone(&tree), &table).expect("answers");
    assert_eq!(arm, Some(Value::Int(big)));
    assert_eq!(interpreted, Some(Value::Float(9_007_199_254_740_991.0)));
    assert!(same(&arm, &interpreted));
}

#[test]
fn an_integer_and_a_float_agree_only_when_the_float_holds_that_exact_integer() {
    assert!(whole_equals_float(
        9_007_199_254_740_992,
        9_007_199_254_740_992.0
    ));
    assert!(!whole_equals_float(
        9_007_199_254_740_993,
        9_007_199_254_740_992.0
    ));
    assert!(whole_equals_float(
        9_007_199_254_740_994,
        9_007_199_254_740_994.0
    ));
    assert!(!whole_equals_float(1, 1.5));
    assert!(!whole_equals_float(1, f64::NAN));
    assert!(!whole_equals_float(1, f64::INFINITY));
    assert!(!whole_equals_float(i64::MAX, 9.223_372_036_854_776e18));
    assert!(whole_equals_float(0, -0.0));
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
