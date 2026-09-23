use std::path::Path;
use std::rc::Rc;

use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::memtable::register_generated_table;
use aqpbm_planeval::df::pre_asap::TableSources;
use aqpbm_planeval::df::pre_asap_arm::{answer_with_tables, PreAsapAnswer};
use aqpbm_planeval::df::schema::declared_columns;
use aqpbm_planeval::df::session::{MemoryPoolSettings, NoSeedBoundFunctions, SeedSession};
use aqpbm_planeval::exact::{run_tree, Data};
use aqpbm_planeval::plan::TimeRangeOrigin;
use aqpbm_planeval::run::RowsFrom;
use aqpbm_planeval::sql::{catalog_from_spec, lower_sql_root_async};
use aqpbm_planeval::Value;
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;
use datafusion::arrow::array::{
    Array, BooleanArray, Float64Array, Int64Array, StringArray, TimestampMillisecondArray,
    UInt64Array,
};
use datafusion::arrow::datatypes::DataType as ArrowDataType;
use datafusion::arrow::record_batch::RecordBatch;

const TABLE: &str = "metrics";

const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

const SPEC: &str = "../configs/datagen/planeval_sql_metrics.yaml";

struct Fixture {
    description: TableDescription,
    table: Rc<GeneratedTable>,
    session: SeedSession,
    tables: TableSources,
}

async fn fixture(rows: u64) -> Fixture {
    let mut description = TableDescription::from_path(Path::new(SPEC)).expect("the spec parses");
    description.row_num = rows;
    let table = Rc::new(description.generate().expect("the spec generates"));
    let session = SeedSession::new(0, MemoryPoolSettings::default(), &NoSeedBoundFunctions)
        .expect("a session needs no OS resources");
    let declared = declared_columns(&description).expect("the spec declares known SQL types");
    register_generated_table(session.context(), TABLE, &table, &declared)
        .expect("the generated table ingests");
    let tables = TableSources::of_context(session.context())
        .await
        .expect("the registered table is reachable");
    Fixture {
        description,
        table,
        session,
        tables,
    }
}

impl Fixture {
    async fn tree(&self, sql: &str) -> Rc<QueryExpr> {
        let catalog = catalog_from_spec(TABLE, &self.description).expect("the spec is a catalog");
        lower_sql_root_async(sql, &catalog, ACCURACY)
            .await
            .expect("the SQL lowers")
    }

    async fn arm(&self, tree: &QueryExpr) -> PreAsapAnswer {
        answer_with_tables(tree, &self.session, &self.tables)
            .await
            .expect("arm A answers")
    }

    fn interpreted(&self, tree: Rc<QueryExpr>) -> Data {
        run_tree(
            tree,
            &RowsFrom::Generated(Rc::clone(&self.table)),
            TimeRangeOrigin::Unknown,
        )
        .expect("ver 1 answers")
        .answer
    }

    async fn text_oracle(&self, sql: &str) -> Vec<RecordBatch> {
        self.session
            .context()
            .sql(sql)
            .await
            .expect("DataFusion parses the text")
            .collect()
            .await
            .expect("DataFusion runs the text")
    }
}

fn values_of(batches: &[RecordBatch]) -> Vec<Vec<Value>> {
    let mut out = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let mut held = Vec::with_capacity(batch.num_columns());
            for column in batch.columns() {
                held.push(value_at(column.as_ref(), row));
            }
            out.push(held);
        }
    }
    out
}

fn value_at(array: &dyn Array, row: usize) -> Value {
    if array.is_null(row) {
        return Value::Null;
    }
    match array.data_type() {
        ArrowDataType::Int64 => Value::Int(
            array
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("an Int64 array")
                .value(row),
        ),
        ArrowDataType::UInt64 => Value::Int(
            array
                .as_any()
                .downcast_ref::<UInt64Array>()
                .expect("a UInt64 array")
                .value(row) as i64,
        ),
        ArrowDataType::Float64 => Value::Float(
            array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("a Float64 array")
                .value(row),
        ),
        ArrowDataType::Utf8 => Value::Str(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a Utf8 array")
                .value(row)
                .to_owned(),
        ),
        ArrowDataType::Boolean => Value::Str(
            array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("a Boolean array")
                .value(row)
                .to_string(),
        ),
        ArrowDataType::Timestamp(_, _) => Value::Timestamp(
            array
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .expect("a millisecond timestamp array")
                .value(row),
        ),
        other => panic!("no ver 1 value corresponds to an Arrow {other}"),
    }
}

fn rows_of(data: &Data) -> Vec<Vec<Value>> {
    match data {
        Data::Rows(rows) => rows.iter().map(|row| row.0.clone()).collect(),
        Data::Scalar(value) => vec![vec![Value::Float(*value)]],
    }
}

fn ordered(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by(|left, right| compare_row(left, right));
    rows
}

fn compare_row(left: &[Value], right: &[Value]) -> std::cmp::Ordering {
    for (left, right) in left.iter().zip(right) {
        let held = compare_value(left, right);
        if held != std::cmp::Ordering::Equal {
            return held;
        }
    }
    left.len().cmp(&right.len())
}

fn compare_value(left: &Value, right: &Value) -> std::cmp::Ordering {
    fn rank(value: &Value) -> u8 {
        match value {
            Value::Null => 0,
            Value::Int(_) => 1,
            Value::Float(_) => 2,
            Value::Str(_) => 3,
            Value::Timestamp(_) => 4,
        }
    }
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => left.cmp(right),
        (Value::Float(left), Value::Float(right)) => left.total_cmp(right),
        (Value::Str(left), Value::Str(right)) => left.cmp(right),
        (Value::Timestamp(left), Value::Timestamp(right)) => left.cmp(right),
        (left, right) => rank(left).cmp(&rank(right)),
    }
}

fn same_number(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left.total_cmp(right).is_eq(),
        (Value::Int(left), Value::Float(right)) | (Value::Float(right), Value::Int(left)) => {
            (*left as f64).total_cmp(right).is_eq()
        }
        (left, right) => compare_value(left, right).is_eq(),
    }
}

fn assert_same_rows(arm: &[Vec<Value>], interpreted: &[Vec<Value>], sql: &str) {
    assert_eq!(
        arm.len(),
        interpreted.len(),
        "{sql}: arm A answered {} rows and ver 1 answered {}",
        arm.len(),
        interpreted.len()
    );
    for (position, (left, right)) in arm.iter().zip(interpreted).enumerate() {
        assert_eq!(
            left.len(),
            right.len(),
            "{sql}: row {position} is {} wide from arm A and {} wide from ver 1",
            left.len(),
            right.len()
        );
        for (column, (left, right)) in left.iter().zip(right).enumerate() {
            assert!(
                same_number(left, right),
                "{sql}: row {position} column {column} is {left:?} from arm A and {right:?} from \
                 ver 1"
            );
        }
    }
}

async fn both_arms(fixture: &Fixture, sql: &str) -> (Vec<Vec<Value>>, Vec<Vec<Value>>) {
    let tree = fixture.tree(sql).await;
    let arm = fixture.arm(&tree).await;
    let interpreted = fixture.interpreted(Rc::clone(&tree));
    (
        ordered(values_of(&arm.batches)),
        ordered(rows_of(&interpreted)),
    )
}

#[tokio::test]
async fn the_milestone_one_query_answers_and_matches_the_interpreted_arm() {
    let fixture = fixture(20_000).await;
    let sql = "SELECT approx_percentile_cont(latency, 0.99) FROM metrics";
    let (arm, interpreted) = both_arms(&fixture, sql).await;
    assert_same_rows(&arm, &interpreted, sql);
}

#[tokio::test]
async fn a_grouped_query_answers_and_matches_the_interpreted_arm() {
    let fixture = fixture(20_000).await;
    let sql = "SELECT service, sum(bytes) FROM metrics GROUP BY service";
    let (arm, interpreted) = both_arms(&fixture, sql).await;
    assert_same_rows(&arm, &interpreted, sql);
}

#[tokio::test]
async fn a_query_with_a_having_clause_answers_and_matches_the_interpreted_arm() {
    let fixture = fixture(20_000).await;
    let unfiltered = "SELECT service, count(*) FROM metrics GROUP BY service";
    let filtered = "SELECT service, count(*) FROM metrics GROUP BY service HAVING count(*) > 2500";
    let (arm, interpreted) = both_arms(&fixture, filtered).await;
    assert_same_rows(&arm, &interpreted, filtered);
    let (groups, _) = both_arms(&fixture, unfiltered).await;
    assert!(
        arm.len() < groups.len(),
        "the clause kept {} of {} groups, so it filtered nothing",
        arm.len(),
        groups.len()
    );
}

#[tokio::test]
async fn every_aggregation_arm_a_runs_collapses_to_one_physical_stage() {
    let fixture = fixture(5_000).await;
    for sql in [
        "SELECT approx_percentile_cont(latency, 0.99) FROM metrics",
        "SELECT service, sum(bytes) FROM metrics GROUP BY service",
        "SELECT service, count(*) FROM metrics GROUP BY service HAVING count(*) > 600",
        "SELECT count(DISTINCT service) FROM metrics",
    ] {
        let tree = fixture.tree(sql).await;
        let text = fixture.arm(&tree).await.physical_text();
        assert!(text.contains("mode=Single"), "{sql}: {text}");
        assert!(!text.contains("mode=Partial"), "{sql}: {text}");
        assert!(!text.contains("mode=Final"), "{sql}: {text}");
    }
}

const CORPUS: [&str; 16] = [
    "SELECT count(*) FROM metrics",
    "SELECT sum(bytes) FROM metrics",
    "SELECT sum(latency) FROM metrics",
    "SELECT avg(latency) FROM metrics",
    "SELECT avg(bytes) FROM metrics",
    "SELECT min(latency), max(latency) FROM metrics",
    "SELECT min(bytes), max(bytes) FROM metrics",
    "SELECT min(ts), max(ts) FROM metrics",
    "SELECT count(DISTINCT service) FROM metrics",
    "SELECT approx_percentile_cont(latency, 0.5) FROM metrics",
    "SELECT approx_percentile_cont(latency, 0.99) FROM metrics",
    "SELECT approx_percentile_cont(bytes, 0.9) FROM metrics",
    "SELECT service, count(*), sum(bytes), avg(latency) FROM metrics GROUP BY service",
    "SELECT service, approx_percentile_cont(latency, 0.95) FROM metrics GROUP BY service",
    "SELECT service, sum(bytes) FROM metrics WHERE latency > 100 GROUP BY service",
    "SELECT service, count(*) FROM metrics GROUP BY service HAVING count(*) > 2500",
];

#[tokio::test]
async fn every_corpus_query_reads_the_same_on_both_arms() {
    let fixture = fixture(20_000).await;
    let mut differing = Vec::new();
    for sql in CORPUS {
        let tree = fixture.tree(sql).await;
        let arm = ordered(values_of(&fixture.arm(&tree).await.batches));
        let interpreted = ordered(rows_of(&fixture.interpreted(Rc::clone(&tree))));
        if arm.len() != interpreted.len() {
            differing.push(format!(
                "{sql}: arm A answered {} rows and ver 1 answered {}",
                arm.len(),
                interpreted.len()
            ));
            continue;
        }
        for (position, (left, right)) in arm.iter().zip(&interpreted).enumerate() {
            for (column, (left, right)) in left.iter().zip(right).enumerate() {
                if !same_number(left, right) {
                    differing.push(format!(
                        "{sql}: row {position} column {column} is {left:?} from arm A and \
                         {right:?} from ver 1"
                    ));
                }
            }
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}

#[tokio::test]
async fn an_exact_quantile_keeps_its_input_type_where_the_interpreted_arm_widens_to_a_float() {
    let fixture = fixture(5_000).await;
    let sql = "SELECT approx_percentile_cont(bytes, 0.9) FROM metrics";
    let (arm, interpreted) = both_arms(&fixture, sql).await;
    assert!(
        matches!(arm[0][0], Value::Int(_)),
        "array_agg keeps the Int64 column: {:?}",
        arm[0][0]
    );
    assert!(
        matches!(interpreted[0][0], Value::Float(_)),
        "ver 1 retains the column as f64 weights: {:?}",
        interpreted[0][0]
    );
    assert!(same_number(&arm[0][0], &interpreted[0][0]));
}

#[tokio::test]
async fn a_text_query_with_no_approximate_function_matches_the_translator_row_for_row() {
    let fixture = fixture(5_000).await;
    let sql = "SELECT service, sum(bytes) FROM metrics GROUP BY service";
    let tree = fixture.tree(sql).await;
    let arm = fixture.arm(&tree).await;
    let text = fixture.text_oracle(sql).await;
    assert_same_rows(
        &ordered(values_of(&arm.batches)),
        &ordered(values_of(&text)),
        sql,
    );
}
