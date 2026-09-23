use asap_types::pre_asap::QueryExpr;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::{collect, displayable, ExecutionPlan};

use crate::df::pre_asap::{lower, TableSources};
use crate::df::session::{SeedSession, SessionError};

pub struct PreAsapAnswer {
    pub logical: LogicalPlan,
    pub physical: std::sync::Arc<dyn ExecutionPlan>,
    pub batches: Vec<RecordBatch>,
    pub reserved_bytes: usize,
}

impl PreAsapAnswer {
    pub fn rows(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }

    pub fn physical_text(&self) -> String {
        displayable(self.physical.as_ref())
            .indent(false)
            .to_string()
    }
}

pub async fn answer(
    expr: &QueryExpr,
    session: &SeedSession,
) -> Result<PreAsapAnswer, SessionError> {
    let tables = TableSources::of_context(session.context()).await?;
    answer_with_tables(expr, session, &tables).await
}

pub async fn answer_with_tables(
    expr: &QueryExpr,
    session: &SeedSession,
    tables: &TableSources,
) -> Result<PreAsapAnswer, SessionError> {
    let logical = lower(expr, tables)?;
    let physical = session.single_mode_physical_plan(&logical).await?;
    let batches = collect(
        std::sync::Arc::clone(&physical),
        session.context().task_ctx(),
    )
    .await?;
    Ok(PreAsapAnswer {
        logical,
        physical,
        batches,
        reserved_bytes: session.reserved_bytes(),
    })
}
