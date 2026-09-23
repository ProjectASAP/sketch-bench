use std::collections::HashMap;
use std::sync::Arc;

use asap_types::post_asap::{PostAsapNodeId, ResultGuarantee};
use datafusion::arrow::compute::concat_batches;
use datafusion::arrow::datatypes::Schema as ArrowSchema;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::{provider_as_source, MemTable, TableProvider};
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder};
use datafusion::physical_plan::{collect, displayable, ExecutionPlan};

use crate::df::post_asap::{fold_order, lower_nodes, DagPlans};
use crate::df::pre_asap::TableSources;
use crate::df::refusal::Refusal;
use crate::df::session::{SeedSession, SessionError};
use crate::df::split::{split, Cut};
use crate::plan::Plan;

pub struct StateTable {
    pub node: PostAsapNodeId,
    pub name: String,
    pub schema: Arc<ArrowSchema>,
    pub logical: LogicalPlan,
    pub physical: Arc<dyn ExecutionPlan>,
    pub batches: Vec<RecordBatch>,
}

impl StateTable {
    pub fn rows(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }

    pub fn bytes(&self) -> usize {
        self.batches
            .iter()
            .map(RecordBatch::get_array_memory_size)
            .sum()
    }
}

pub struct PostAsapAnswer {
    pub state_tables: Vec<StateTable>,
    pub logical: LogicalPlan,
    pub physical: Arc<dyn ExecutionPlan>,
    pub batches: Vec<RecordBatch>,
    pub no_summary_in_plan: bool,
    pub readout_guarantees: Vec<(PostAsapNodeId, ResultGuarantee)>,
}

impl PostAsapAnswer {
    pub fn rows(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }

    pub fn physical_text(&self) -> String {
        displayable(self.physical.as_ref())
            .indent(false)
            .to_string()
    }

    pub fn state_bytes(&self) -> usize {
        self.state_tables.iter().map(StateTable::bytes).sum()
    }
}

pub async fn answer(
    plan: &Plan,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<PostAsapAnswer, SessionError> {
    let parts = split(plan)?;
    let mut scans: HashMap<PostAsapNodeId, LogicalPlan> = HashMap::new();
    let mut state_tables = Vec::with_capacity(parts.cuts.len());

    for cut in &parts.cuts {
        let (table, held) = materialize(cut, plan, session, tables).await?;
        scans.insert(cut.producer, scan_of(&table.name, held)?);
        state_tables.push(table);
    }

    let read = lower_nodes(
        &plan.dag,
        &parts.read_nodes,
        tables,
        &session.state(),
        &scans,
    )?;
    let logical = read.plan(plan.dag.root)?.clone();
    let (physical, batches) = run(&logical, session).await?;

    Ok(PostAsapAnswer {
        state_tables,
        logical,
        physical,
        batches,
        no_summary_in_plan: parts.no_summary_in_plan,
        readout_guarantees: read.readout_guarantees().to_vec(),
    })
}

pub async fn answer_without_split(
    plan: &Plan,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<PostAsapAnswer, SessionError> {
    let parts = split(plan)?;
    let order = fold_order(&plan.dag)?;
    let whole: DagPlans =
        lower_nodes(&plan.dag, &order, tables, &session.state(), &HashMap::new())?;
    let logical = whole.plan(plan.dag.root)?.clone();
    let (physical, batches) = run(&logical, session).await?;

    Ok(PostAsapAnswer {
        state_tables: Vec::new(),
        logical,
        physical,
        batches,
        no_summary_in_plan: parts.no_summary_in_plan,
        readout_guarantees: whole.readout_guarantees().to_vec(),
    })
}

async fn materialize(
    cut: &Cut,
    plan: &Plan,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<(StateTable, Arc<MemTable>), SessionError> {
    let built = lower_nodes(
        &plan.dag,
        &cut.maintenance_nodes,
        tables,
        &session.state(),
        &HashMap::new(),
    )?;
    let logical = built.plan(cut.producer)?.clone();
    let (physical, batches) = run(&logical, session).await?;

    let schema = Arc::new(cut.state_table_schema(physical.schema().as_ref())?);
    let retyped = batches
        .iter()
        .map(|batch| {
            RecordBatch::try_new(Arc::clone(&schema), batch.columns().to_vec())
                .map_err(DataFusionError::from)
        })
        .collect::<Result<Vec<_>, DataFusionError>>()?;
    let held = Arc::new(MemTable::try_new(
        Arc::clone(&schema),
        vec![retyped.clone()],
    )?);
    session.context().register_table(
        cut.table.as_str(),
        Arc::clone(&held) as Arc<dyn TableProvider>,
    )?;

    Ok((
        StateTable {
            node: cut.producer,
            name: cut.table.clone(),
            schema,
            logical,
            physical,
            batches: retyped,
        },
        held,
    ))
}

fn scan_of(name: &str, held: Arc<MemTable>) -> Result<LogicalPlan, SessionError> {
    Ok(LogicalPlanBuilder::scan(
        name,
        provider_as_source(held as Arc<dyn TableProvider>),
        None,
    )
    .and_then(LogicalPlanBuilder::build)?)
}

async fn run(
    logical: &LogicalPlan,
    session: &SeedSession,
) -> Result<(Arc<dyn ExecutionPlan>, Vec<RecordBatch>), SessionError> {
    let physical = session.single_mode_physical_plan(logical).await?;
    let batches = collect(Arc::clone(&physical), session.context().task_ctx()).await?;
    Ok((physical, batches))
}

pub type NamedColumnBuffers = Vec<(String, Vec<Vec<u8>>)>;

pub fn column_bytes(batches: &[RecordBatch]) -> Result<NamedColumnBuffers, Refusal> {
    let Some(first) = batches.first() else {
        return Ok(Vec::new());
    };
    let schema = first.schema();
    let joined = concat_batches(&schema, batches).map_err(|error| {
        Refusal::no_constructor(
            "RecordBatch",
            format!("the answer's batches do not concatenate: {error}"),
        )
    })?;
    Ok(joined
        .schema()
        .fields()
        .iter()
        .zip(joined.columns())
        .map(|(field, column)| {
            let data = column.to_data();
            let mut buffers: Vec<Vec<u8>> =
                vec![format!("{}|{}|{}", data.data_type(), data.len(), data.offset()).into_bytes()];
            if let Some(nulls) = data.nulls() {
                buffers.push(nulls.validity().to_vec());
            }
            for buffer in data.buffers() {
                buffers.push(buffer.as_slice().to_vec());
            }
            (field.name().clone(), buffers)
        })
        .collect())
}

pub fn refuse_unless_answers_are_identical(
    split_run: &[RecordBatch],
    whole_run: &[RecordBatch],
) -> Result<(), Refusal> {
    if column_bytes(split_run)? != column_bytes(whole_run)? {
        return Err(Refusal::no_constructor(
            "SplitAgreement",
            "the split run and the whole-graph run do not hand back the same bytes",
        ));
    }
    Ok(())
}
