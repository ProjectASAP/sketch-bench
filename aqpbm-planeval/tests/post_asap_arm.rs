use std::path::Path;
use std::rc::Rc;

use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::memtable::register_generated_table;
use aqpbm_planeval::df::post_asap_arm::{
    answer, answer_without_split, column_bytes, PostAsapAnswer,
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

fn scalars(batches: &[RecordBatch], column: usize) -> Vec<f64> {
    let mut held = Vec::new();
    for batch in batches {
        let array = batch.column(column);
        if let Some(floats) = array.as_any().downcast_ref::<Float64Array>() {
            for row in 0..floats.len() {
                held.push(floats.value(row));
            }
        } else if let Some(integers) = array.as_any().downcast_ref::<Int64Array>() {
            for row in 0..integers.len() {
                held.push(integers.value(row) as f64);
            }
        } else {
            panic!("column {column} is {:?}, not a number", array.data_type());
        }
    }
    held
}

fn keys(batches: &[RecordBatch], column: usize) -> Vec<String> {
    let mut held = Vec::new();
    for batch in batches {
        let array = batch
            .column(column)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("a Utf8 column");
        for row in 0..array.len() {
            held.push(array.value(row).to_owned());
        }
    }
    held
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
    assert_eq!(read, vec![expected], "arm B and ver 1 read the same bits");
}

#[tokio::test]
async fn the_quantile_query_answers_the_same_split_and_whole() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(QUANTILE_SQL).await;
    let split_run = fixture.split_answer(&plan).await;
    let whole_run = fixture.whole_answer(&plan).await;
    assert_eq!(
        column_bytes(&split_run.batches).unwrap(),
        column_bytes(&whole_run.batches).unwrap(),
        "the splitter's own correctness test"
    );
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
    assert_eq!(
        column_bytes(&split_run.batches).unwrap(),
        column_bytes(&whole_run.batches).unwrap(),
        "the splitter's own correctness test"
    );

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

    let mut mine: Vec<(String, f64)> = keys(&split_run.batches, 0)
        .into_iter()
        .zip(scalars(&split_run.batches, 1))
        .collect();
    let mut theirs: Vec<(String, f64)> =
        keys(&text, 0).into_iter().zip(scalars(&text, 1)).collect();
    mine.sort_by(|left, right| left.0.cmp(&right.0));
    theirs.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(mine, theirs);
}

#[tokio::test]
async fn the_grouped_sum_is_the_one_ver_one_computes_under_the_same_seed() {
    let fixture = fixture(20_000).await;
    let plan = fixture.plan(GROUPED_SUM_SQL).await;
    let split_run = fixture.split_answer(&plan).await;

    let mut mine: Vec<(String, f64)> = keys(&split_run.batches, 0)
        .into_iter()
        .zip(scalars(&split_run.batches, 1))
        .collect();
    let mut theirs: Vec<(String, f64)> = fixture
        .interpreted_rows(&plan)
        .into_iter()
        .map(|Row(values)| {
            let Value::Str(service) = &values[0] else {
                panic!("the group key is a string: {values:?}");
            };
            (
                service.clone(),
                values[1].as_f64().expect("the sum is a number"),
            )
        })
        .collect();
    mine.sort_by(|left, right| left.0.cmp(&right.0));
    theirs.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(mine, theirs, "arm B and ver 1 read the same bits");
}
