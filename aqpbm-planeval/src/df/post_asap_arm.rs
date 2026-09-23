use std::collections::HashMap;
use std::sync::Arc;

use asap_types::post_asap::{PostAsapNodeId, ResultGuarantee};
use datafusion::arrow::array::ArrayData;
use datafusion::arrow::compute::concat_batches;
use datafusion::arrow::datatypes::{Field, Schema as ArrowSchema};
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
    pub maintenance_peak_reserved_bytes: usize,
    pub read_peak_reserved_bytes: usize,
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
    let mut maintenance_peak_reserved_bytes = 0usize;

    for cut in &parts.cuts {
        let (table, held, peak) = materialize(cut, plan, session, tables).await?;
        maintenance_peak_reserved_bytes = maintenance_peak_reserved_bytes.max(peak);
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
    let (physical, batches, read_peak_reserved_bytes) = run(&logical, session).await?;

    Ok(PostAsapAnswer {
        state_tables,
        logical,
        physical,
        batches,
        no_summary_in_plan: parts.no_summary_in_plan,
        readout_guarantees: read.readout_guarantees().to_vec(),
        maintenance_peak_reserved_bytes,
        read_peak_reserved_bytes,
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
    let (physical, batches, peak) = run(&logical, session).await?;

    Ok(PostAsapAnswer {
        state_tables: Vec::new(),
        logical,
        physical,
        batches,
        no_summary_in_plan: parts.no_summary_in_plan,
        readout_guarantees: whole.readout_guarantees().to_vec(),
        maintenance_peak_reserved_bytes: 0,
        read_peak_reserved_bytes: peak,
    })
}

async fn materialize(
    cut: &Cut,
    plan: &Plan,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<(StateTable, Arc<MemTable>, usize), SessionError> {
    let built = lower_nodes(
        &plan.dag,
        &cut.maintenance_nodes,
        tables,
        &session.state(),
        &HashMap::new(),
    )?;
    let logical = built.plan(cut.producer)?.clone();
    let (physical, batches, peak) = run(&logical, session).await?;

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
        peak,
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
) -> Result<(Arc<dyn ExecutionPlan>, Vec<RecordBatch>, usize), SessionError> {
    let physical = session.single_mode_physical_plan(logical).await?;
    session.forget_peak_reserved_bytes();
    let batches = collect(Arc::clone(&physical), session.context().task_ctx()).await?;
    Ok((physical, batches, session.peak_reserved_bytes()))
}

pub type NamedColumnBuffers = Vec<(String, Vec<Vec<u8>>)>;

fn column_bytes(batches: &[RecordBatch]) -> Result<NamedColumnBuffers, Refusal> {
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
            let mut buffers = Vec::new();
            append_array_bytes(&column.to_data(), &mut buffers);
            (field.name().clone(), buffers)
        })
        .collect())
}

fn append_array_bytes(data: &ArrayData, buffers: &mut Vec<Vec<u8>>) {
    buffers.push(format!("{}|{}|{}", data.data_type(), data.len(), data.offset()).into_bytes());
    if let Some(nulls) = data.nulls() {
        buffers.push(nulls.validity().to_vec());
    }
    for buffer in data.buffers() {
        buffers.push(buffer.as_slice().to_vec());
    }
    for child in data.child_data() {
        append_array_bytes(child, buffers);
    }
}

fn field_text(field: &Field) -> String {
    format!("{} {}", field.name(), field.data_type())
}

fn refuse_unless_schemas_are_identical(
    split_run: &ArrowSchema,
    whole_run: &ArrowSchema,
) -> Result<(), Refusal> {
    if split_run.fields().len() != whole_run.fields().len() {
        return Err(Refusal::no_constructor(
            "SplitAgreement",
            format!(
                "the split run answers {} columns and the whole-graph run answers {}",
                split_run.fields().len(),
                whole_run.fields().len()
            ),
        ));
    }
    for (position, (split_field, whole_field)) in split_run
        .fields()
        .iter()
        .zip(whole_run.fields())
        .enumerate()
    {
        if split_field.name() != whole_field.name()
            || split_field.data_type() != whole_field.data_type()
        {
            return Err(Refusal::no_constructor(
                "SplitAgreement",
                format!(
                    "column {position} is {} from the split run and {} from the whole-graph run",
                    field_text(split_field),
                    field_text(whole_field)
                ),
            ));
        }
    }
    Ok(())
}

fn refuse_unless_columns_are_identical(
    split_run: &[RecordBatch],
    whole_run: &[RecordBatch],
) -> Result<(), Refusal> {
    let split_columns = column_bytes(split_run)?;
    let whole_columns = column_bytes(whole_run)?;
    if split_columns.len() != whole_columns.len() {
        return Err(Refusal::no_constructor(
            "SplitAgreement",
            format!(
                "the split run hands back {} columns of bytes and the whole-graph run hands back \
                 {}",
                split_columns.len(),
                whole_columns.len()
            ),
        ));
    }
    for ((split_name, split_bytes), (whole_name, whole_bytes)) in
        split_columns.iter().zip(&whole_columns)
    {
        if split_name != whole_name {
            return Err(Refusal::no_constructor(
                "SplitAgreement",
                format!(
                    "the split run names this column {split_name} and the whole-graph run names \
                     it {whole_name}"
                ),
            ));
        }
        if split_bytes != whole_bytes {
            return Err(Refusal::no_constructor(
                "SplitAgreement",
                format!(
                    "column {split_name} does not hold the same bytes in the split run and the \
                     whole-graph run"
                ),
            ));
        }
    }
    Ok(())
}

pub fn refuse_unless_answers_are_identical(
    split_run: &PostAsapAnswer,
    whole_run: &PostAsapAnswer,
) -> Result<(), Refusal> {
    refuse_unless_schemas_are_identical(
        split_run.physical.schema().as_ref(),
        whole_run.physical.schema().as_ref(),
    )?;
    refuse_unless_columns_are_identical(&split_run.batches, &whole_run.batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{
        ArrayRef, DictionaryArray, Int32Array, Int64Array, ListArray, StringArray, StructArray,
    };
    use datafusion::arrow::buffer::OffsetBuffer;
    use datafusion::arrow::datatypes::{DataType as ArrowDataType, Fields, Int32Type};

    fn top_k_entries() -> Fields {
        vec![
            Field::new("key", ArrowDataType::Utf8, false),
            Field::new("count", ArrowDataType::Int64, false),
        ]
        .into()
    }

    fn top_k_column(keys: [&str; 2], counts: [i64; 2]) -> ArrayRef {
        let fields = top_k_entries();
        let entries = StructArray::new(
            fields.clone(),
            vec![
                Arc::new(StringArray::from(keys.to_vec())) as ArrayRef,
                Arc::new(Int64Array::from(counts.to_vec())) as ArrayRef,
            ],
            None,
        );
        Arc::new(ListArray::new(
            Arc::new(Field::new("item", ArrowDataType::Struct(fields), false)),
            OffsetBuffer::new(vec![0, 2].into()),
            Arc::new(entries),
            None,
        ))
    }

    fn one_column_batch(name: &str, column: ArrayRef) -> Vec<RecordBatch> {
        let schema = Arc::new(ArrowSchema::new(vec![Field::new(
            name,
            column.data_type().clone(),
            false,
        )]));
        vec![RecordBatch::try_new(schema, vec![column]).expect("one column is a batch")]
    }

    #[test]
    fn a_top_k_readout_that_ranks_other_keys_is_not_the_same_answer() {
        let ranked = one_column_batch("top", top_k_column(["a", "b"], [10, 9]));
        let other = one_column_batch("top", top_k_column(["z", "y"], [1, 0]));
        assert_eq!(
            column_bytes(&ranked).unwrap()[0].1.len(),
            column_bytes(&other).unwrap()[0].1.len(),
            "the two answers have the same shape, so only the values tell them apart"
        );
        refuse_unless_columns_are_identical(&ranked, &ranked)
            .expect("an answer is the same as itself");
        let refused = refuse_unless_columns_are_identical(&ranked, &other).unwrap_err();
        assert_eq!(refused.variant, "SplitAgreement");
    }

    #[test]
    fn a_dictionary_column_that_spells_its_keys_differently_is_not_the_same_answer() {
        let keys = Int32Array::from(vec![0, 1]);
        let spelled: ArrayRef = Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                keys.clone(),
                Arc::new(StringArray::from(vec!["a", "b"])),
            )
            .expect("two keys index two values"),
        );
        let other: ArrayRef = Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                keys,
                Arc::new(StringArray::from(vec!["c", "d"])),
            )
            .expect("two keys index two values"),
        );
        let refused = refuse_unless_columns_are_identical(
            &one_column_batch("service", spelled),
            &one_column_batch("service", other),
        )
        .unwrap_err();
        assert_eq!(refused.variant, "SplitAgreement");
    }

    #[test]
    fn two_answers_that_hold_no_batch_still_have_their_schemas_compared() {
        let counted = ArrowSchema::new(vec![Field::new("n", ArrowDataType::Int64, false)]);
        let renamed = ArrowSchema::new(vec![Field::new("service", ArrowDataType::Int64, false)]);
        let retyped = ArrowSchema::new(vec![Field::new("n", ArrowDataType::Float64, false)]);
        let two = ArrowSchema::new(vec![
            Field::new("n", ArrowDataType::Int64, false),
            Field::new("service", ArrowDataType::Utf8, false),
        ]);
        let widened = ArrowSchema::new(vec![Field::new("n", ArrowDataType::Int64, true)]);

        refuse_unless_columns_are_identical(&[], &[]).expect("no bytes disagree with no bytes");
        refuse_unless_schemas_are_identical(&counted, &counted).expect("a schema is itself");
        for disagreeing in [&renamed, &retyped, &two] {
            let refused = refuse_unless_schemas_are_identical(&counted, disagreeing).unwrap_err();
            assert_eq!(refused.variant, "SplitAgreement");
        }
        refuse_unless_schemas_are_identical(&counted, &widened).expect(
            "the split run reads a state table that declares the IR's own nullability and the \
             whole-graph run reads DataFusion's inference of it, so only the validity the rows \
             carry tells the two answers apart",
        );
    }
}
