use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use datafusion::execution::context::{QueryPlanner, SessionState};
use datafusion::execution::memory_pool::{GreedyMemoryPool, MemoryPool, TrackConsumersPool};
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::logical_expr::{AggregateUDF, LogicalPlan, ScalarUDF, UserDefinedLogicalNode};
use datafusion::physical_plan::aggregates::{AggregateExec, AggregateMode};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner};
use datafusion::prelude::{SessionConfig, SessionContext};

use crate::df::refusal::Refusal;

pub const TARGET_PARTITIONS: usize = 1;

pub const DEFAULT_TRACKED_CONSUMERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPoolSettings {
    pub limit_bytes: usize,
    pub tracked_consumers: NonZeroUsize,
}

impl Default for MemoryPoolSettings {
    fn default() -> Self {
        Self {
            limit_bytes: usize::MAX,
            tracked_consumers: NonZeroUsize::new(DEFAULT_TRACKED_CONSUMERS)
                .expect("DEFAULT_TRACKED_CONSUMERS is not zero"),
        }
    }
}

pub trait SeedBoundFunctions: Send + Sync {
    fn aggregates(&self, seed: u64) -> Vec<AggregateUDF>;
    fn scalars(&self, seed: u64) -> Vec<ScalarUDF>;
}

pub struct NoSeedBoundFunctions;

impl SeedBoundFunctions for NoSeedBoundFunctions {
    fn aggregates(&self, _seed: u64) -> Vec<AggregateUDF> {
        Vec::new()
    }

    fn scalars(&self, _seed: u64) -> Vec<ScalarUDF> {
        Vec::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    DataFusion(#[from] DataFusionError),
}

#[derive(Debug)]
pub struct RefusingExtensionPlanner;

#[async_trait]
impl ExtensionPlanner for RefusingExtensionPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        _logical_inputs: &[&LogicalPlan],
        _physical_inputs: &[Arc<dyn ExecutionPlan>],
        _session_state: &SessionState,
    ) -> DataFusionResult<Option<Arc<dyn ExecutionPlan>>> {
        Err(Refusal::deferred(
            format!("LogicalPlan::Extension({})", node.name()),
            "no-extension-operator",
            "this session registers no physical operator for an extension node",
        )
        .into())
    }
}

pub struct ExtensionQueryPlanner {
    physical_planner: DefaultPhysicalPlanner,
}

impl std::fmt::Debug for ExtensionQueryPlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionQueryPlanner")
            .field("extension_planners", &[RefusingExtensionPlanner])
            .finish()
    }
}

impl Default for ExtensionQueryPlanner {
    fn default() -> Self {
        Self {
            physical_planner: DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(
                RefusingExtensionPlanner,
            )]),
        }
    }
}

#[async_trait]
impl QueryPlanner for ExtensionQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session_state: &SessionState,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.physical_planner
            .create_physical_plan(logical_plan, session_state)
            .await
    }
}

pub struct SeedSession {
    seed: u64,
    context: SessionContext,
    memory_pool: Arc<dyn MemoryPool>,
}

impl SeedSession {
    pub fn new(
        seed: u64,
        settings: MemoryPoolSettings,
        functions: &dyn SeedBoundFunctions,
    ) -> Result<Self, SessionError> {
        let memory_pool: Arc<dyn MemoryPool> = Arc::new(TrackConsumersPool::new(
            GreedyMemoryPool::new(settings.limit_bytes),
            settings.tracked_consumers,
        ));
        let runtime = RuntimeEnvBuilder::new()
            .with_memory_pool(Arc::clone(&memory_pool))
            .build_arc()?;
        let config = SessionConfig::new().with_target_partitions(TARGET_PARTITIONS);
        let state = SessionStateBuilder::new()
            .with_config(config)
            .with_runtime_env(runtime)
            .with_default_features()
            .with_query_planner(Arc::new(ExtensionQueryPlanner::default()))
            .build();
        let context = SessionContext::new_with_state(state);

        for aggregate in functions.aggregates(seed) {
            context.register_udaf(aggregate);
        }
        for scalar in functions.scalars(seed) {
            context.register_udf(scalar);
        }

        Ok(Self {
            seed,
            context,
            memory_pool,
        })
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn context(&self) -> &SessionContext {
        &self.context
    }

    pub fn state(&self) -> SessionState {
        self.context.state()
    }

    pub fn reserved_bytes(&self) -> usize {
        self.memory_pool.reserved()
    }

    pub async fn single_mode_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
    ) -> Result<Arc<dyn ExecutionPlan>, SessionError> {
        let state = self.state();
        let plan = state.create_physical_plan(logical_plan).await?;
        refuse_unless_aggregates_are_single(plan.as_ref())?;
        Ok(plan)
    }
}

pub fn sessions_for_seeds(
    seeds: impl IntoIterator<Item = u64>,
    settings: MemoryPoolSettings,
    functions: &dyn SeedBoundFunctions,
) -> Result<Vec<SeedSession>, SessionError> {
    seeds
        .into_iter()
        .map(|seed| SeedSession::new(seed, settings, functions))
        .collect()
}

pub fn refuse_unless_aggregates_are_single(plan: &dyn ExecutionPlan) -> Result<(), Refusal> {
    if let Some(aggregate) = plan.as_any().downcast_ref::<AggregateExec>() {
        let mode = *aggregate.mode();
        if mode != AggregateMode::Single {
            return Err(Refusal::deferred(
                format!("AggregateExec({mode:?})"),
                "multi-partition-aggregate",
                format!(
                    "at target_partitions = {TARGET_PARTITIONS} the physical plan must carry one \
                     {:?} aggregate, not a {mode:?} stage whose merge order the answer depends on",
                    AggregateMode::Single
                ),
            ));
        }
    }
    for child in plan.children() {
        refuse_unless_aggregates_are_single(child.as_ref())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Float64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType as ArrowDataType, Field, Schema as ArrowSchema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::datasource::MemTable;
    use datafusion::logical_expr::{Extension, UserDefinedLogicalNodeCore};
    use datafusion::physical_plan::displayable;
    use std::cmp::Ordering;
    use std::fmt;

    fn session() -> SeedSession {
        SeedSession::new(0, MemoryPoolSettings::default(), &NoSeedBoundFunctions).unwrap()
    }

    fn register_two_column_table(session: &SeedSession) {
        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("service", ArrowDataType::Utf8, false),
            Field::new("latency", ArrowDataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "a", "b"])),
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])),
            ],
        )
        .unwrap();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        session
            .context()
            .register_table("t", Arc::new(table))
            .unwrap();
    }

    #[derive(PartialEq, Eq, Hash)]
    struct NoSuchNode {
        schema: datafusion::common::DFSchemaRef,
    }

    impl fmt::Debug for NoSuchNode {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            UserDefinedLogicalNodeCore::fmt_for_explain(self, f)
        }
    }

    impl PartialOrd for NoSuchNode {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl UserDefinedLogicalNodeCore for NoSuchNode {
        fn name(&self) -> &str {
            "NoSuchNode"
        }

        fn inputs(&self) -> Vec<&LogicalPlan> {
            Vec::new()
        }

        fn schema(&self) -> &datafusion::common::DFSchemaRef {
            &self.schema
        }

        fn expressions(&self) -> Vec<datafusion::logical_expr::Expr> {
            Vec::new()
        }

        fn fmt_for_explain(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "NoSuchNode")
        }

        fn with_exprs_and_inputs(
            &self,
            _exprs: Vec<datafusion::logical_expr::Expr>,
            _inputs: Vec<LogicalPlan>,
        ) -> DataFusionResult<Self> {
            Ok(Self {
                schema: Arc::clone(&self.schema),
            })
        }
    }

    impl Ord for NoSuchNode {
        fn cmp(&self, _other: &Self) -> Ordering {
            Ordering::Equal
        }
    }

    #[tokio::test]
    async fn the_session_plans_at_one_partition() {
        let session = session();
        assert_eq!(
            session.state().config().target_partitions(),
            TARGET_PARTITIONS
        );
    }

    #[tokio::test]
    async fn a_grouped_aggregation_comes_out_as_a_single_stage() {
        let session = session();
        register_two_column_table(&session);
        let frame = session
            .context()
            .sql("SELECT service, sum(latency) FROM t GROUP BY service")
            .await
            .unwrap();
        let logical = frame.logical_plan().clone();
        let physical = session.single_mode_physical_plan(&logical).await.unwrap();
        let text = displayable(physical.as_ref()).indent(false).to_string();
        assert!(text.contains("mode=Single"), "{text}");
        assert!(!text.contains("mode=Partial"), "{text}");
    }

    #[tokio::test]
    async fn a_partial_aggregate_is_refused_by_mode() {
        let session = session();
        register_two_column_table(&session);
        let frame = session
            .context()
            .sql("SELECT service, sum(latency) FROM t GROUP BY service")
            .await
            .unwrap();
        let single = session
            .state()
            .create_physical_plan(&frame.logical_plan().clone())
            .await
            .unwrap();
        let aggregate = single
            .as_any()
            .downcast_ref::<AggregateExec>()
            .expect("the root of this plan is the aggregate");
        let partial = AggregateExec::try_new(
            AggregateMode::Partial,
            aggregate.group_expr().clone(),
            aggregate.aggr_expr().to_vec(),
            aggregate.filter_expr().to_vec(),
            Arc::clone(&aggregate.input().clone()),
            aggregate.input_schema(),
        )
        .unwrap();
        let refused = refuse_unless_aggregates_are_single(&partial).unwrap_err();
        assert_eq!(refused.variant, "AggregateExec(Partial)");
        assert_eq!(refused.tag(), "deferred");
    }

    #[tokio::test]
    async fn an_extension_node_is_refused_by_name() {
        let session = session();
        let schema = Arc::new(
            datafusion::common::DFSchema::try_from(ArrowSchema::new(vec![Field::new(
                "n",
                ArrowDataType::Int64,
                false,
            )]))
            .unwrap(),
        );
        let logical = LogicalPlan::Extension(Extension {
            node: Arc::new(NoSuchNode { schema }),
        });
        let error = session
            .state()
            .create_physical_plan(&logical)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("NoSuchNode"),
            "the refusal names the node: {error}"
        );
        assert!(
            error.to_string().contains("no-extension-operator"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn one_session_per_seed_and_each_keeps_its_own_pool() {
        use crate::df::sketch_udaf::{sketch_aggregates, SummaryFunctions, KLL_FUNCTION};

        let bare =
            sessions_for_seeds(0..3, MemoryPoolSettings::default(), &NoSeedBoundFunctions).unwrap();
        let seeds: Vec<u64> = bare.iter().map(SeedSession::seed).collect();
        assert_eq!(seeds, vec![0, 1, 2]);
        for session in &bare {
            assert_eq!(session.reserved_bytes(), 0);
            let registered = session.state().aggregate_functions().clone();
            assert!(
                registered.contains_key("sum"),
                "DataFusion's own aggregates are there whether or not anything registers"
            );
            assert!(
                !registered.contains_key(KLL_FUNCTION),
                "a session bound to no functions holds none of ours"
            );
        }

        let bound =
            sessions_for_seeds(0..3, MemoryPoolSettings::default(), &SummaryFunctions).unwrap();
        let built_in: Vec<String> = bare[0]
            .state()
            .aggregate_functions()
            .keys()
            .cloned()
            .collect();
        for session in &bound {
            assert_eq!(session.reserved_bytes(), 0);
            let registered = session.state().aggregate_functions().clone();
            let ours: Vec<&String> = registered
                .keys()
                .filter(|name| !built_in.contains(name))
                .collect();
            let expected: Vec<String> = sketch_aggregates(session.seed())
                .iter()
                .map(|function| function.name().to_owned())
                .collect();
            assert_eq!(ours.len(), expected.len(), "{ours:?}");
            for name in &expected {
                assert!(registered.contains_key(name), "{name}");
            }
            let kll = registered.get(KLL_FUNCTION).expect("the KLL aggregate");
            assert!(
                format!("{kll:?}").contains(&format!("seed: {}", session.seed())),
                "the registered instance carries this session's seed: {kll:?}"
            );
        }
    }
}
