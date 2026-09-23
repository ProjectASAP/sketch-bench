use std::path::Path;
use std::rc::Rc;

use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::memtable::register_generated_table;
use aqpbm_planeval::df::metrics::{node_of_alias, ElapsedCompute, NODE_ALIAS_PREFIX};
use aqpbm_planeval::df::post_asap_arm::answer;
use aqpbm_planeval::df::pre_asap::TableSources;
use aqpbm_planeval::df::pre_asap_arm::answer_with_tables;
use aqpbm_planeval::df::run::{run as run_datafusion, DataFusionRunConfig, ANSWER_ERROR_METRIC};
use aqpbm_planeval::df::schema::declared_columns;
use aqpbm_planeval::df::session::{MemoryPoolSettings, SeedSession};
use aqpbm_planeval::df::sketch_udaf::SummaryFunctions;
use aqpbm_planeval::df::split::split;
use aqpbm_planeval::plan::{plan_sql, plan_sql_root, Plan};
use aqpbm_planeval::record::PlanEvalRecord;
use aqpbm_planeval::run::{RowsFrom, RunConfig};
use aqpbm_planeval::score::ObservedError;
use aqpbm_planeval::sql::{catalog_from_spec, lower_sql_root_async};
use asap_types::post_asap::PostAsapNodeId;
use asap_types::types::AccuracyTarget;

const TABLE: &str = "metrics";
const SPEC: &str = "../configs/datagen/planeval_sql_metrics.yaml";
const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);
const SEED: u64 = 0;
const ROWS: u64 = 20_000;

const QUANTILE_SQL: &str = "SELECT approx_percentile_cont(latency, 0.99) FROM metrics";
const GROUPED_SUM_SQL: &str = "SELECT service, SUM(bytes) FROM metrics GROUP BY service";
const NO_SUMMARY_SQL: &str = "SELECT latency FROM metrics WHERE latency > 1";

fn description() -> TableDescription {
    let mut description = TableDescription::from_path(Path::new(SPEC)).expect("the spec parses");
    description.row_num = ROWS;
    description
}

fn table(description: &TableDescription) -> Rc<GeneratedTable> {
    Rc::new(description.generate().expect("the spec generates"))
}

fn session_over(description: &TableDescription, rows: &GeneratedTable) -> SeedSession {
    let session = SeedSession::new(SEED, MemoryPoolSettings::default(), &SummaryFunctions)
        .expect("the session registers the summary functions");
    let declared = declared_columns(description).expect("the spec declares known SQL types");
    register_generated_table(session.context(), TABLE, rows, &declared)
        .expect("the generated table ingests");
    session
}

async fn plan_of(sql: &str, description: &TableDescription) -> Plan {
    let catalog = catalog_from_spec(TABLE, description).expect("the spec is a catalog");
    let root = lower_sql_root_async(sql, &catalog, ACCURACY)
        .await
        .expect("the SQL lowers");
    plan_sql_root(sql, root).expect("the tree plans")
}

fn engine(
    description: &TableDescription,
    rows: &Rc<GeneratedTable>,
    split: bool,
) -> DataFusionRunConfig {
    DataFusionRunConfig {
        table: TABLE.to_string(),
        description: description.clone(),
        rows: Rc::clone(rows),
        split,
    }
}

fn record_of(sql: &str, split: bool) -> PlanEvalRecord {
    let description = description();
    let rows = table(&description);
    let catalog = catalog_from_spec(TABLE, &description).expect("the spec is a catalog");
    let plan = plan_sql(sql, &catalog, ACCURACY).expect("the SQL plans");
    let config = RunConfig::new(RowsFrom::Generated(Rc::clone(&rows)), SEED, true);
    let outcome = run_datafusion(&plan, &config, &engine(&description, &rows, split))
        .expect("both arms run on DataFusion");
    PlanEvalRecord::from_run(sql, &plan, &outcome)
}

#[tokio::test]
async fn each_node_that_projects_or_aggregates_names_itself_in_the_physical_plan() {
    let description = description();
    let rows = table(&description);
    let session = session_over(&description, &rows);
    let tables = TableSources::of_context(session.context()).await.unwrap();
    let plan = plan_of(QUANTILE_SQL, &description).await;
    let run = answer(&plan, &session, &tables)
        .await
        .expect("arm B answers");

    let maintenance = datafusion::physical_plan::displayable(run.state_tables[0].physical.as_ref())
        .indent(false)
        .to_string();
    assert!(
        maintenance.contains(&format!("{NODE_ALIAS_PREFIX}1_")),
        "the SummaryAgg's aggregate carries its node: {maintenance}"
    );

    let mut charged = ElapsedCompute::default();
    charged.add_plan(run.state_tables[0].physical.as_ref());
    charged.add_plan(run.physical.as_ref());
    assert!(
        charged.per_node.contains_key(&PostAsapNodeId(1)),
        "the sketch build is charged to the node that declared it: {charged:?}"
    );
    assert!(
        charged.attributed_operators >= 2,
        "both queries attribute at least their own operator: {charged:?}"
    );
    assert!(charged.operators() > charged.attributed_operators);
}

#[tokio::test]
async fn the_state_column_keeps_the_name_the_edge_declares_under_the_alias() {
    let description = description();
    let rows = table(&description);
    let session = session_over(&description, &rows);
    let tables = TableSources::of_context(session.context()).await.unwrap();
    let plan = plan_of(QUANTILE_SQL, &description).await;
    let run = answer(&plan, &session, &tables)
        .await
        .expect("arm B answers");

    let state = &run.state_tables[0];
    let named = state.schema.field(0).name();
    assert_eq!(
        node_of_alias(named),
        None,
        "the state table is written under the edge's own name, not the alias: {named}"
    );
    let produced = state.physical.schema();
    assert_eq!(
        node_of_alias(produced.field(0).name()),
        Some(state.node),
        "the query that fills it names the producing node"
    );
}

#[tokio::test]
async fn the_memory_number_is_a_peak_taken_while_the_query_runs() {
    let description = description();
    let rows = table(&description);
    let session = session_over(&description, &rows);
    let tables = TableSources::of_context(session.context()).await.unwrap();
    let plan = plan_of(QUANTILE_SQL, &description).await;
    let tree = plan.pre_asap.as_ref().expect("the SQL has a pre-ASAP tree");

    let arm = answer_with_tables(tree, &session, &tables)
        .await
        .expect("arm A answers");
    assert!(
        arm.peak_reserved_bytes > 0,
        "the exact quantile holds the whole column while it sorts it"
    );
    assert_eq!(
        session.reserved_bytes(),
        0,
        "and by the time collect hands the batches over the pool is empty again, which is why \
         the peak cannot be read here"
    );
}

#[tokio::test]
async fn a_plan_the_planner_left_whole_is_not_counted_as_a_summary_win() {
    let description = description();
    let plan = plan_of(NO_SUMMARY_SQL, &description).await;
    let parts = split(&plan).expect("the plan splits");
    assert!(parts.cuts.is_empty(), "nothing is summarized");
    assert!(
        !parts.is_an_advantage(),
        "every node runs at read time, so there is no summary to have an advantage over"
    );
    assert!(parts.no_summary_in_plan);
}

#[test]
fn the_quantile_query_reports_four_numbers_on_datafusion() {
    let record = record_of(QUANTILE_SQL, true);
    assert_eq!(record.runtime, "datafusion");
    assert_eq!(record.refusals.total(), 0);

    let maintenance = record
        .approximate
        .maintenance
        .as_ref()
        .expect("the maintenance query is timed");
    let read = record
        .approximate
        .read
        .as_ref()
        .expect("the read query is timed");
    assert_eq!(maintenance.work, ROWS);
    assert!(maintenance.elapsed_ms.mean > 0.0);
    assert!(read.elapsed_ms.mean > 0.0);
    assert!(
        record.approximate.build.is_none(),
        "DataFusion binds nothing"
    );

    assert!(
        record.approximate.state_bytes > 0,
        "the state table holds the sketch"
    );
    assert!(record.pre_asap.retained_bytes > record.approximate.state_bytes);
    assert_eq!(
        record.approximate.peak_reserved_bytes,
        Some(0),
        "an ungrouped aggregate over an accumulator that does not grow reserves nothing from \
         DataFusion 43's pool; the state table is where this sketch's size shows up"
    );
    assert!(record.pre_asap.peak_reserved_bytes.unwrap() > 0);

    let advantage = record.advantage();
    assert!(advantage.aggregate_time.unwrap() > 0.0);
    assert!(
        advantage.query_time.unwrap() > 1.0,
        "reading a KLL beats sorting the column"
    );
    assert!(advantage.memory.unwrap() > 1.0);
    let accuracy = advantage.accuracy.expect("both arms answered");
    assert!((0.0..1.0).contains(&accuracy), "{accuracy}");

    let readout = &record.readouts[0];
    assert!(
        matches!(&readout.observed_error, ObservedError::Measured { metric, .. }
            if metric == ANSWER_ERROR_METRIC)
    );
    assert_eq!(
        readout.claimed_bound, None,
        "the node's bound is over ranks and this error is over values"
    );
}

#[test]
fn the_grouped_sum_is_exact_on_both_arms() {
    let record = record_of(GROUPED_SUM_SQL, true);
    assert_eq!(record.runtime, "datafusion");
    assert!(record.approximate.maintenance.is_some());
    assert!(
        record.approximate.peak_reserved_bytes.unwrap() > 0,
        "a grouped aggregate resizes its reservation as the hash table grows"
    );
    assert!(
        record.readouts.is_empty(),
        "a plan with no SummaryEstimate has nothing to score"
    );
    assert_eq!(record.advantage().accuracy, None);
}

#[test]
fn running_the_graph_whole_charges_everything_to_the_read_query() {
    let split_run = record_of(QUANTILE_SQL, true);
    let whole_run = record_of(QUANTILE_SQL, false);

    assert!(whole_run.approximate.maintenance.is_none());
    assert!(whole_run.approximate.read.is_some());
    assert_eq!(
        whole_run.approximate.aggregate_ms(),
        0.0,
        "an uncut graph has no maintenance query to charge"
    );
    assert_eq!(
        whole_run.approximate.state_bytes, 0,
        "and no state table, because nothing was materialized between the two halves"
    );
    assert_eq!(
        whole_run.readouts[0].approximate, split_run.readouts[0].approximate,
        "cutting the graph does not change the answer"
    );
}
