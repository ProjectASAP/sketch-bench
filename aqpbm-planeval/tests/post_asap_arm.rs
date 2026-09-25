use std::path::Path;
use std::rc::Rc;

use aqpbm_datagen::column::ColumnSpec;
use aqpbm_datagen::dist::{DataDistribution, UniformParameter};
use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::memtable::register_generated_table;
use aqpbm_planeval::df::post_asap_arm::{
    answer, answer_without_split, refuse_unless_answers_are_identical, PostAsapAnswer,
};
use aqpbm_planeval::df::pre_asap::TableSources;
use aqpbm_planeval::df::schema::declared_columns;
use aqpbm_planeval::df::session::{MemoryPoolSettings, SeedSession};
use aqpbm_planeval::df::sketch_udaf::SummaryFunctions;
use aqpbm_planeval::df::split::split;
use aqpbm_planeval::exact::Data;
use aqpbm_planeval::plan::{plan_sql_root, Plan};
use aqpbm_planeval::run::{run, RowsFrom, RunConfig, RunOutcome};
use aqpbm_planeval::sql::{catalog_from_spec, lower_sql_root_async};
use aqpbm_planeval::types::{Answer, Row};
use aqpbm_planeval::Value;
use asap_types::types::AccuracyTarget;
use datafusion::arrow::array::{Array, Float64Array, Int64Array, StringArray};
use datafusion::arrow::datatypes::DataType as ArrowDataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::physical_plan::displayable;

const TABLE: &str = "metrics";

const SPEC: &str = "../configs/datagen/planeval_sql_metrics.yaml";

const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

const SEED: u64 = 0;

const QUANTILE_SQL: &str = "SELECT approx_percentile_cont(latency, 0.99) FROM metrics";

const GROUPED_SUM_SQL: &str = "SELECT service, SUM(bytes) FROM metrics GROUP BY service";

const LARGE_COUNT: &str = "large_count";

const LARGE_GROUPED_SUM_SQL: &str =
    "SELECT service, SUM(large_count) FROM metrics GROUP BY service";

struct Fixture {
    description: TableDescription,
    table: Rc<GeneratedTable>,
    session: SeedSession,
    tables: TableSources,
}

async fn fixture(rows: u64) -> Fixture {
    let mut description = TableDescription::from_path(Path::new(SPEC)).expect("the spec parses");
    description.row_num = rows;
    fixture_of(description).await
}

async fn fixture_that_sums_past_exactly_representable_floats(rows: u64) -> Fixture {
    let mut description = TableDescription::from_path(Path::new(SPEC)).expect("the spec parses");
    description.row_num = rows;
    description.column_num += 1;
    description.column_label.push(LARGE_COUNT.to_owned());
    description.column_spec.push(ColumnSpec {
        distribution: DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: 1000.0,
            seed: 5,
        }),
        shift: Some(1.0e14),
        cardinality: None,
        special_rule: 0,
        data_type: "i64".to_owned(),
        sql_type: None,
        string: None,
    });
    fixture_of(description).await
}

async fn fixture_of(description: TableDescription) -> Fixture {
    let table = Rc::new(description.generate().expect("the spec generates"));
    let session = SeedSession::new(SEED, MemoryPoolSettings::default(), &SummaryFunctions)
        .expect("the session registers the summary functions");
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
    async fn plan(&self, sql: &str) -> Plan {
        let catalog = catalog_from_spec(TABLE, &self.description).expect("the spec is a catalog");
        let root = lower_sql_root_async(sql, &catalog, ACCURACY)
            .await
            .expect("the SQL lowers");
        plan_sql_root(sql, root).expect("the tree plans")
    }

    async fn split_answer(&self, plan: &Plan) -> PostAsapAnswer {
        answer(plan, &self.session, &self.tables)
            .await
            .expect("arm B answers")
    }

    async fn whole_answer(&self, plan: &Plan) -> PostAsapAnswer {
        answer_without_split(plan, &self.session, &self.tables)
            .await
            .expect("the whole graph answers")
    }

    fn interpreted(&self, plan: &Plan) -> Vec<Answer> {
        self.interpreted_run(plan)
            .readouts
            .into_iter()
            .map(|readout| readout.approximate)
            .collect()
    }

    fn interpreted_rows(&self, plan: &Plan) -> Vec<Row> {
        match self
            .interpreted_run(plan)
            .pre_asap_answer
            .expect("ver 1 runs the pre-ASAP arm of the same plan")
        {
            Data::Rows(rows) => (*rows).clone(),
            Data::Scalar(value) => vec![Row(vec![Value::Float(value)])],
        }
    }

    fn interpreted_run(&self, plan: &Plan) -> RunOutcome {
        let config = RunConfig::new(RowsFrom::Generated(Rc::clone(&self.table)), SEED, false);
        run(plan, &config).expect("ver 1 runs the same plan")
    }
}

fn scalars(batches: &[RecordBatch], column: usize) -> Vec<Value> {
    let mut held = Vec::new();
    for batch in batches {
        let array = batch.column(column);
        if let Some(floats) = array.as_any().downcast_ref::<Float64Array>() {
            for row in 0..floats.len() {
                held.push(if floats.is_null(row) {
                    Value::Null
                } else {
                    Value::Float(floats.value(row))
                });
            }
        } else if let Some(integers) = array.as_any().downcast_ref::<Int64Array>() {
            for row in 0..integers.len() {
                held.push(if integers.is_null(row) {
                    Value::Null
                } else {
                    Value::Int(integers.value(row))
                });
            }
        } else {
            panic!("column {column} is {:?}, not a number", array.data_type());
        }
    }
    held
}

fn keys(batches: &[RecordBatch], column: usize) -> Vec<Value> {
    let mut held = Vec::new();
    for batch in batches {
        let array = batch
            .column(column)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("a Utf8 column");
        for row in 0..array.len() {
            held.push(if array.is_null(row) {
                Value::Null
            } else {
                Value::Str(array.value(row).to_owned())
            });
        }
    }
    held
}

fn same_number(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Int(left), Value::Int(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left.total_cmp(right).is_eq(),
        (Value::Int(left), Value::Float(right)) | (Value::Float(right), Value::Int(left)) => {
            whole_equals_float(*left, *right)
        }
        _ => false,
    }
}

fn whole_equals_float(whole: i64, float: f64) -> bool {
    if !float.is_finite() || float.fract() != 0.0 {
        return false;
    }
    let exact = float as i128;
    exact as f64 == float && exact == i128::from(whole)
}

fn key_text(value: &Value) -> Option<&str> {
    match value {
        Value::Str(held) => Some(held.as_str()),
        Value::Null => None,
        other => panic!("the group key is a string or a null: {other:?}"),
    }
}

fn by_key(rows: Vec<(Value, Value)>) -> Vec<(Value, Value)> {
    let mut held = rows;
    held.sort_by(|left, right| key_text(&left.0).cmp(&key_text(&right.0)));
    held
}

fn assert_same_groups(mine: Vec<(Value, Value)>, theirs: Vec<(Value, Value)>, what: &str) {
    let mine = by_key(mine);
    let theirs = by_key(theirs);
    assert_eq!(
        mine.len(),
        theirs.len(),
        "{what}: arm B answered {} groups and the oracle answered {}",
        mine.len(),
        theirs.len()
    );
    for (position, (mine, theirs)) in mine.iter().zip(&theirs).enumerate() {
        assert_eq!(
            mine.0, theirs.0,
            "{what}: group {position} is a different key"
        );
        assert!(
            same_number(&mine.1, &theirs.1),
            "{what}: group {position} ({:?}) is {:?} from arm B and {:?} from the oracle",
            mine.0,
            mine.1,
            theirs.1
        );
    }
}

#[tokio::test]
async fn the_quantile_query_cuts_one_state_table_and_reads_it_back() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(QUANTILE_SQL).await;
    let parts = split(&plan).expect("the plan splits");
    assert_eq!(parts.cuts.len(), 1, "one SummaryAgg feeds one readout");
    assert!(parts.is_an_advantage(), "the plan holds a summary");
    assert!(
        parts.cuts[0].table.starts_with("__asap_state_"),
        "{}",
        parts.cuts[0].table
    );

    let split_run = fixture.split_answer(&plan).await;
    assert_eq!(split_run.state_tables.len(), 1);
    assert_eq!(split_run.state_tables[0].rows(), 1);
    assert_eq!(
        split_run.state_tables[0].schema.field(0).data_type(),
        &ArrowDataType::Binary,
        "a sketch state column is Binary"
    );
    assert_eq!(split_run.rows(), 1);

    let maintenance = displayable(split_run.state_tables[0].physical.as_ref())
        .indent(false)
        .to_string();
    assert!(maintenance.contains("mode=Single"), "{maintenance}");
    assert!(!maintenance.contains("mode=Partial"), "{maintenance}");
    let built = split_run.state_tables[0]
        .logical
        .display_indent()
        .to_string();
    assert!(
        built.contains("asap_sketch_kll"),
        "the maintenance query builds the sketch: {built}"
    );

    let read = split_run.physical_text();
    assert!(
        read.contains("asap_estimate_kll_quantile"),
        "the read query reads the state back: {read}"
    );
    assert!(
        !read.contains("AggregateExec"),
        "the read query holds no aggregate: {read}"
    );

    let whole = fixture.whole_answer(&plan).await;
    let whole_text = whole.physical_text();
    assert!(whole_text.contains("mode=Single"), "{whole_text}");
    assert!(!whole_text.contains("mode=Partial"), "{whole_text}");
}

#[tokio::test]
async fn the_quantile_readout_is_the_one_ver_one_computes_under_the_same_seed() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(QUANTILE_SQL).await;
    let split_run = fixture.split_answer(&plan).await;
    let read = scalars(&split_run.batches, 0);
    let interpreted = fixture.interpreted(&plan);
    assert_eq!(interpreted.len(), 1, "one readout");
    let Answer::Scalar(expected) = interpreted[0] else {
        panic!("a quantile reads out as a scalar");
    };
    assert_eq!(read.len(), 1, "one row");
    assert!(
        same_number(&read[0], &Value::Float(expected)),
        "arm B read {:?} and ver 1 read {expected}",
        read[0]
    );
}

#[tokio::test]
async fn the_quantile_query_answers_the_same_split_and_whole() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(QUANTILE_SQL).await;
    let split_run = fixture.split_answer(&plan).await;
    let whole_run = fixture.whole_answer(&plan).await;
    refuse_unless_answers_are_identical(&split_run, &whole_run)
        .expect("the splitter's own correctness test");
}

#[tokio::test]
async fn the_grouped_sum_cuts_one_state_table_that_keeps_the_summarized_column_type() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(GROUPED_SUM_SQL).await;
    let parts = split(&plan).expect("the plan splits");
    assert_eq!(parts.cuts.len(), 1, "one ExactAggregate feeds one read");
    assert!(parts.is_an_advantage());

    let split_run = fixture.split_answer(&plan).await;
    assert_eq!(split_run.state_tables.len(), 1);
    let state = &split_run.state_tables[0];
    assert_eq!(
        state.schema.field(0).data_type(),
        &ArrowDataType::Utf8,
        "the group key keeps its own type"
    );
    assert_eq!(
        state.schema.field(1).data_type(),
        &ArrowDataType::Int64,
        "an exact Sum accumulator keeps the summarized column's type"
    );
    assert_eq!(state.rows(), 8, "the spec draws eight service names");

    let whole_run = fixture.whole_answer(&plan).await;
    refuse_unless_answers_are_identical(&split_run, &whole_run)
        .expect("the splitter's own correctness test");

    let services = keys(&split_run.batches, 0);
    let sums = scalars(&split_run.batches, 1);
    assert_eq!(services.len(), sums.len());
    assert_eq!(services.len(), 8);
}

#[tokio::test]
async fn the_grouped_sum_matches_what_datafusion_computes_from_the_text() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(GROUPED_SUM_SQL).await;
    let split_run = fixture.split_answer(&plan).await;

    let text = fixture
        .session
        .context()
        .sql(GROUPED_SUM_SQL)
        .await
        .expect("DataFusion parses the text")
        .collect()
        .await
        .expect("DataFusion runs the text");

    let mine: Vec<(Value, Value)> = keys(&split_run.batches, 0)
        .into_iter()
        .zip(scalars(&split_run.batches, 1))
        .collect();
    let theirs: Vec<(Value, Value)> = keys(&text, 0).into_iter().zip(scalars(&text, 1)).collect();
    assert_same_groups(mine, theirs, GROUPED_SUM_SQL);
}

#[tokio::test]
async fn the_grouped_sum_is_the_one_ver_one_computes_under_the_same_seed() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(GROUPED_SUM_SQL).await;
    let split_run = fixture.split_answer(&plan).await;

    let mine: Vec<(Value, Value)> = keys(&split_run.batches, 0)
        .into_iter()
        .zip(scalars(&split_run.batches, 1))
        .collect();
    let theirs: Vec<(Value, Value)> = fixture
        .interpreted_rows(&plan)
        .into_iter()
        .map(|Row(values)| (values[0].clone(), values[1].clone()))
        .collect();
    assert_same_groups(mine, theirs, GROUPED_SUM_SQL);
}

#[test]
fn two_sums_that_differ_above_two_to_the_fifty_third_are_not_the_same_number() {
    let held = 1_i64 << 53;
    let past = held + 1;
    assert_eq!(
        held as f64, past as f64,
        "the two sums are one f64 once either side is cast"
    );
    assert!(!same_number(&Value::Int(held), &Value::Int(past)));
    assert!(!same_number(&Value::Int(past), &Value::Float(past as f64)));
    assert!(same_number(&Value::Int(held), &Value::Float(held as f64)));
    assert!(same_number(&Value::Int(past), &Value::Int(past)));
    assert!(!same_number(&Value::Null, &Value::Int(0)));
    assert!(!same_number(&Value::Null, &Value::Float(0.0)));
}

#[tokio::test]
async fn a_grouped_sum_above_two_to_the_fifty_third_is_the_one_ver_one_computes() {
    let fixture = fixture_that_sums_past_exactly_representable_floats(20_000).await;
    let plan = fixture.plan(LARGE_GROUPED_SUM_SQL).await;
    let split_run = fixture.split_answer(&plan).await;

    let sums = scalars(&split_run.batches, 1);
    assert_eq!(sums.len(), 8, "the spec draws eight service names");
    let past_exact = sums.iter().filter(|sum| match sum {
        Value::Int(held) => *held > (1_i64 << 53) && *held as f64 as i64 != *held,
        other => panic!("an exact Sum of an i64 column reads out as an integer: {other:?}"),
    });
    assert!(
        past_exact.count() > 0,
        "at least one group sums to an integer no f64 holds: {sums:?}"
    );

    let mine: Vec<(Value, Value)> = keys(&split_run.batches, 0).into_iter().zip(sums).collect();
    let theirs: Vec<(Value, Value)> = fixture
        .interpreted_rows(&plan)
        .into_iter()
        .map(|Row(values)| (values[0].clone(), values[1].clone()))
        .collect();
    assert_same_groups(mine, theirs, LARGE_GROUPED_SUM_SQL);

    let whole_run = fixture.whole_answer(&plan).await;
    refuse_unless_answers_are_identical(&split_run, &whole_run)
        .expect("the splitter's own correctness test");
}
