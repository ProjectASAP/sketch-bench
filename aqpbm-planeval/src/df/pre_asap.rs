use std::collections::BTreeMap;
use std::sync::Arc;

use asap_types::pre_asap::query_expr::{JoinKind, Reduction, Source};
use asap_types::pre_asap::schema::Schema;
use asap_types::pre_asap::QueryExpr;
use datafusion::datasource::provider_as_source;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{
    col, Expr, JoinType, LogicalPlan, LogicalPlanBuilder, SortExpr, TableSource,
};
use datafusion::prelude::SessionContext;
use datafusion::sql::TableReference;

use crate::df::agg_intent::lower_measure;
use crate::df::refusal::Refusal;
use crate::df::scalar::{lower_scalar, ColumnScope};
use crate::df::schema::arrow_type;

pub const MEASURE_STATE_PREFIX: &str = "__asap_m";

pub const SCALAR_ROOT_COLUMN: &str = "value";

#[derive(Clone, Default)]
pub struct TableSources {
    sources: BTreeMap<String, Arc<dyn TableSource>>,
}

impl TableSources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_table(mut self, name: impl Into<String>, source: Arc<dyn TableSource>) -> Self {
        self.sources.insert(name.into(), source);
        self
    }

    pub async fn of_context(context: &SessionContext) -> Result<Self, DataFusionError> {
        let state = context.state();
        let catalog_name = state.config_options().catalog.default_catalog.clone();
        let schema_name = state.config_options().catalog.default_schema.clone();
        let catalog = context.catalog(&catalog_name).ok_or_else(|| {
            DataFusionError::Plan(format!("catalog {catalog_name} is not registered"))
        })?;
        let schema = catalog.schema(&schema_name).ok_or_else(|| {
            DataFusionError::Plan(format!("schema {schema_name} is not registered"))
        })?;
        let mut sources = BTreeMap::new();
        for name in schema.table_names() {
            if let Some(provider) = schema.table(&name).await? {
                sources.insert(name, provider_as_source(provider));
            }
        }
        Ok(Self { sources })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    pub fn source(&self, table_ref: &str) -> Result<Arc<dyn TableSource>, Refusal> {
        self.sources
            .get(table_ref)
            .or_else(|| {
                let bare = TableReference::parse_str(table_ref);
                self.sources.get(bare.table())
            })
            .cloned()
            .ok_or_else(|| {
                Refusal::no_constructor(
                    "QueryExpr::Scan",
                    format!(
                        "no table named {table_ref} is registered; this session holds [{}]",
                        self.names().collect::<Vec<_>>().join(", ")
                    ),
                )
            })
    }
}

pub fn lower(expr: &QueryExpr, tables: &TableSources) -> Result<LogicalPlan, Refusal> {
    match expr {
        QueryExpr::Scan {
            source,
            predicates,
            schema,
        } => lower_scan(source, predicates, schema, tables),

        QueryExpr::Filter { pred, child } => {
            let input = lower(child, tables)?;
            let scope = ColumnScope::of_plan(&input);
            let pred = lower_scalar(&pred.0, &scope)?;
            build(LogicalPlanBuilder::from(input).filter(pred), "Filter")
        }

        QueryExpr::Project {
            cols,
            qualifier,
            child,
        } => {
            let input = lower(child, tables)?;
            let scope = ColumnScope::of_plan(&input);
            let names = output_columns(expr)?;
            let mut projection = Vec::with_capacity(cols.len());
            for (position, item) in cols.iter().enumerate() {
                let name = names
                    .get(position)
                    .map(|column| column.name.clone())
                    .ok_or_else(|| {
                        Refusal::no_constructor(
                            "QueryExpr::Project",
                            format!("item {position} has no column in the declared output schema"),
                        )
                    })?;
                projection.push(lower_scalar(&item.expr, &scope)?.alias(name));
            }
            let projected = build(
                LogicalPlanBuilder::from(input).project(projection),
                "Project",
            )?;
            match qualifier {
                None => Ok(projected),
                Some(alias) => build(
                    LogicalPlanBuilder::from(projected).alias(TableReference::bare(alias.clone())),
                    "Project",
                ),
            }
        }

        QueryExpr::Aggregate {
            reduction,
            measures,
            having,
            child,
            ..
        } => lower_aggregate(expr, reduction, measures, having.as_ref(), child, tables),

        QueryExpr::Sort {
            keys,
            partition_by,
            child,
        } => {
            if !partition_by.is_empty() {
                return Err(Refusal::deferred(
                    "QueryExpr::Sort",
                    "sort-partition-limit-coupling",
                    format!(
                        "an order partitioned by {:?} only means anything together with the Limit \
                         above it, and the IR carries no coupling between the two; translating \
                         the order alone reports a plausible wrong answer",
                        partition_by.keys()
                    ),
                ));
            }
            let input = lower(child, tables)?;
            let scope = ColumnScope::of_plan(&input);
            let mut sorts = Vec::with_capacity(keys.len());
            for key in keys {
                sorts.push(SortExpr::new(
                    lower_scalar(&key.expr, &scope)?,
                    key.ascending,
                    key.nulls_first,
                ));
            }
            build(LogicalPlanBuilder::from(input).sort(sorts), "Sort")
        }

        QueryExpr::Limit { n, offset, child } => {
            let input = lower(child, tables)?;
            build(
                LogicalPlanBuilder::from(input).limit(*offset, Some(*n)),
                "Limit",
            )
        }

        QueryExpr::Concat { children, .. } => {
            let mut branches = children.iter();
            let Some(first) = branches.next() else {
                return Err(Refusal::no_constructor(
                    "QueryExpr::Concat",
                    "an empty branch list derives no schema, and upstream rejects it at \
                     construction",
                ));
            };
            let mut held = LogicalPlanBuilder::from(lower(first, tables)?);
            for branch in branches {
                held = build(held.union(lower(branch, tables)?), "Concat")
                    .map(LogicalPlanBuilder::from)?;
            }
            build(Ok(held), "Concat")
        }

        QueryExpr::Join {
            kind,
            pred,
            left,
            right,
        } => {
            let join_type = match kind {
                JoinKind::Inner => JoinType::Inner,
                JoinKind::Left => JoinType::Left,
                other => {
                    return Err(Refusal::deferred(
                        "QueryExpr::Join",
                        "join-kind",
                        format!(
                            "{other:?} is a join shape no corpus query reaches yet; this step \
                             translates Inner and Left"
                        ),
                    ))
                }
            };
            let left_plan = lower(left, tables)?;
            let right_plan = lower(right, tables)?;
            let scope = ColumnScope::joined(
                &ColumnScope::of_plan(&left_plan),
                &ColumnScope::of_plan(&right_plan),
            );
            let condition = lower_scalar(&pred.0, &scope)?;
            build(
                LogicalPlanBuilder::from(left_plan).join_on(right_plan, join_type, [condition]),
                "Join",
            )
        }

        QueryExpr::PromqlScalarBridge(inner) => one_row(
            lower_scalar(inner, &ColumnScope::empty())?,
            "PromqlScalarBridge",
        ),

        QueryExpr::Literal(_) => one_row(lower_scalar(expr, &ColumnScope::empty())?, "Literal"),

        QueryExpr::Dedup { .. } => Err(Refusal::deferred(
            "QueryExpr::Dedup",
            "corpus",
            "SELECT DISTINCT ON has a DataFusion node, and no corpus query reaches it yet",
        )),
        QueryExpr::SetOp { .. } => Err(Refusal::deferred(
            "QueryExpr::SetOp",
            "corpus",
            "UNION / INTERSECT / EXCEPT have DataFusion nodes, and no corpus query reaches them \
             yet",
        )),
        QueryExpr::SQLWindowFunc { .. } => Err(Refusal::deferred(
            "QueryExpr::SQLWindowFunc",
            "corpus",
            "the fifteen window kinds map to DataFusion's built-in window functions, and no \
             corpus query reaches them yet",
        )),
        QueryExpr::BinaryOp { vector_match, .. } => {
            if vector_match.is_some() {
                Err(Refusal::promql_only(
                    "QueryExpr::BinaryOp",
                    "vector matching pairs series by label set, which SQL has no shape for",
                ))
            } else {
                Err(Refusal::deferred(
                    "QueryExpr::BinaryOp",
                    "corpus",
                    "two single-row operands compose as a cross join under one projection, and no \
                     corpus query reaches that shape yet",
                ))
            }
        }

        QueryExpr::EvalTimestamp => Err(no_evaluation_instant("QueryExpr::EvalTimestamp")),
        QueryExpr::CurrentTimestamp => Err(no_evaluation_instant("QueryExpr::CurrentTimestamp")),
        QueryExpr::TimeRange { .. } => Err(no_evaluation_instant("QueryExpr::TimeRange")),
        QueryExpr::TimeShift { .. } => Err(no_evaluation_instant("QueryExpr::TimeShift")),
        QueryExpr::PromqlSubquery { .. } => Err(no_evaluation_instant("QueryExpr::PromqlSubquery")),

        QueryExpr::PromqlVectorFromScalar(_) => {
            Err(promql_operator("QueryExpr::PromqlVectorFromScalar"))
        }
        QueryExpr::PromqlScalarFromVector(_) => {
            Err(promql_operator("QueryExpr::PromqlScalarFromVector"))
        }
        QueryExpr::PromqlRelabel { .. } => Err(promql_operator("QueryExpr::PromqlRelabel")),
        QueryExpr::PromqlInfoEnrich { .. } => Err(promql_operator("QueryExpr::PromqlInfoEnrich")),
        QueryExpr::PromqlSeriesSample { .. } => {
            Err(promql_operator("QueryExpr::PromqlSeriesSample"))
        }

        QueryExpr::Column(_)
        | QueryExpr::Compare { .. }
        | QueryExpr::BoolAnd(_)
        | QueryExpr::BoolOr(_)
        | QueryExpr::Not(_)
        | QueryExpr::IsNull(_)
        | QueryExpr::IsNotNull(_)
        | QueryExpr::Cast { .. }
        | QueryExpr::InList { .. }
        | QueryExpr::FunctionCall { .. }
        | QueryExpr::Arithmetic { .. }
        | QueryExpr::Case { .. } => Err(scalar_in_operator_position(expr)),
    }
}

fn lower_scan(
    source: &Source,
    predicates: &[asap_types::pre_asap::query_expr::Predicate],
    schema: &Schema,
    tables: &TableSources,
) -> Result<LogicalPlan, Refusal> {
    let table_ref = match source {
        Source::Table { table_ref } => table_ref,
        Source::TimeSeries { metric } => {
            return Err(Refusal::promql_only(
                "QueryExpr::Scan",
                format!(
                    "a time-series leaf ({metric}) has no registered table; PromQL input keeps \
                     running on the interpreted runtime"
                ),
            ))
        }
    };
    let table = tables.source(table_ref)?;
    check_leaf_schema(table_ref, schema, table.schema().as_ref())?;
    let scan = build(
        LogicalPlanBuilder::scan(TableReference::bare(table_ref.clone()), table, None),
        "Scan",
    )?;
    if predicates.is_empty() {
        return Ok(scan);
    }
    let scope = ColumnScope::of_plan(&scan);
    let mut held: Option<Expr> = None;
    for predicate in predicates {
        let lowered = lower_scalar(&predicate.0, &scope)?;
        held = Some(match held {
            None => lowered,
            Some(previous) => previous.and(lowered),
        });
    }
    match held {
        None => Ok(scan),
        Some(pred) => build(LogicalPlanBuilder::from(scan).filter(pred), "Scan"),
    }
}

fn check_leaf_schema(
    table_ref: &str,
    declared: &Schema,
    registered: &datafusion::arrow::datatypes::Schema,
) -> Result<(), Refusal> {
    if declared.columns.len() != registered.fields().len() {
        return Err(Refusal::no_constructor(
            "QueryExpr::Scan",
            format!(
                "the bound schema of {table_ref} names {} columns and the registered table holds \
                 {}",
                declared.columns.len(),
                registered.fields().len()
            ),
        ));
    }
    for (position, (column, field)) in declared
        .columns
        .iter()
        .zip(registered.fields().iter())
        .enumerate()
    {
        if column.name != *field.name() {
            return Err(Refusal::no_constructor(
                "QueryExpr::Scan",
                format!(
                    "column {position} of {table_ref} is {} in the bound schema and {} in the \
                     registered table",
                    column.name,
                    field.name()
                ),
            ));
        }
        let wanted = arrow_type(&column.dtype)?;
        if wanted != *field.data_type() {
            return Err(Refusal::no_constructor(
                "QueryExpr::Scan",
                format!(
                    "column {} of {table_ref} is {:?} in the bound schema and {} in the \
                     registered table",
                    column.name,
                    column.dtype,
                    field.data_type()
                ),
            ));
        }
        if column.nullable != field.is_nullable() {
            return Err(Refusal::no_constructor(
                "QueryExpr::Scan",
                format!(
                    "column {} of {table_ref} is {} in the bound schema and {} in the registered \
                     table",
                    column.name,
                    nullability(column.nullable),
                    nullability(field.is_nullable())
                ),
            ));
        }
    }
    Ok(())
}

fn nullability(nullable: bool) -> &'static str {
    if nullable {
        "nullable"
    } else {
        "not nullable"
    }
}

fn lower_aggregate(
    node: &QueryExpr,
    reduction: &Reduction,
    measures: &[asap_types::pre_asap::agg_intent::AggIntent],
    having: Option<&asap_types::pre_asap::query_expr::Predicate>,
    child: &QueryExpr,
    tables: &TableSources,
) -> Result<LogicalPlan, Refusal> {
    let by =
        match reduction {
            Reduction::PerEntity => {
                return Err(Refusal::time_axis(
                    "QueryExpr::Aggregate",
                    "a per-entity reduction runs along each series' own time axis, and the DAG \
                 carries no evaluation instant",
                ))
            }
            Reduction::Reduce(by) if by.is_without() => return Err(Refusal::promql_only(
                "QueryExpr::Aggregate",
                "GROUP BY every column except these has no SQL spelling, and the SQL front end \
                 never emits it",
            )),
            Reduction::Reduce(by) => by,
        };

    let input = lower(child, tables)?;
    let scope = ColumnScope::of_plan(&input);
    let declared = output_columns(node)?;
    if declared.len() != by.keys().len() + measures.len() {
        return Err(Refusal::no_constructor(
            "QueryExpr::Aggregate",
            format!(
                "the declared output schema names {} columns and this node reduces {} keys and {} \
                 measures",
                declared.len(),
                by.keys().len(),
                measures.len()
            ),
        ));
    }

    let mut group = Vec::with_capacity(by.keys().len());
    for key in by.keys() {
        group.push(scope.expr(*key)?);
    }

    let mut aggregated = Vec::with_capacity(measures.len());
    let mut readouts = Vec::with_capacity(measures.len());
    for (position, measure) in measures.iter().enumerate() {
        let state = format!("{MEASURE_STATE_PREFIX}{position}");
        let lowered = lower_measure(measure, &scope, &state)?;
        aggregated.push(lowered.aggregated.alias(state.clone()));
        readouts.push(lowered.readout.unwrap_or_else(|| col(state)));
    }

    let reduced = build(
        LogicalPlanBuilder::from(input).aggregate(group.clone(), aggregated),
        "Aggregate",
    )?;
    let reduced_scope = ColumnScope::of_plan(&reduced);

    let mut projection = Vec::with_capacity(declared.len());
    for (position, column) in declared.iter().take(group.len()).enumerate() {
        projection.push(reduced_scope.expr(position)?.alias(column.name.clone()));
    }
    for (readout, column) in readouts.into_iter().zip(declared.iter().skip(group.len())) {
        projection.push(readout.alias(column.name.clone()));
    }
    let named = build(
        LogicalPlanBuilder::from(reduced).project(projection),
        "Aggregate",
    )?;

    let Some(having) = having else {
        return Ok(named);
    };
    let having_scope = ColumnScope::of_plan(&named);
    let pred = lower_scalar(&having.0, &having_scope)?;
    build(LogicalPlanBuilder::from(named).filter(pred), "Aggregate")
}

fn output_columns(node: &QueryExpr) -> Result<Vec<asap_types::pre_asap::schema::Column>, Refusal> {
    node.output_schema()
        .map(|schema| schema.columns)
        .map_err(|error| {
            Refusal::no_constructor(
                format!("QueryExpr::{}", crate::rows::variant_name(node)),
                format!("the node declares no output schema: {error}"),
            )
        })
}

fn one_row(value: Expr, variant: &str) -> Result<LogicalPlan, Refusal> {
    build(
        LogicalPlanBuilder::empty(true).project([value.alias(SCALAR_ROOT_COLUMN)]),
        variant,
    )
}

fn build(
    builder: Result<LogicalPlanBuilder, DataFusionError>,
    variant: &str,
) -> Result<LogicalPlan, Refusal> {
    builder
        .and_then(LogicalPlanBuilder::build)
        .map_err(|error| {
            Refusal::deferred(
                format!("QueryExpr::{variant}"),
                "logical-plan-construction",
                format!("DataFusion declined to build this node: {error}"),
            )
        })
}

pub fn scalar_in_operator_position(expr: &QueryExpr) -> Refusal {
    Refusal::no_constructor(
        format!("QueryExpr::{}", crate::rows::variant_name(expr)),
        "an operator position takes a relation; this variant produces one value per row and \
         upstream issue #205 dropped the type-level split that used to keep it out",
    )
}

fn no_evaluation_instant(variant: &str) -> Refusal {
    Refusal::time_axis(
        variant,
        "the node is defined relative to the moment the query is evaluated, and the DAG carries \
         no such instant",
    )
}

fn promql_operator(variant: &str) -> Refusal {
    Refusal::promql_only(
        variant,
        "the node exists to serve a PromQL construct the SQL front end never lowers to",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::df::refusal::RefusalReason;
    use crate::df::variants::{
        every_query_expr, leaf_arrow_schema, scan, vector_matched_binary_op, BYTES, LATENCY,
        SERVICE, TABLE,
    };
    use asap_types::pre_asap::agg_intent::AggIntent;
    use asap_types::pre_asap::expr_ir::{CompareOpKind, ScalarValue};
    use asap_types::pre_asap::query_expr::{GroupKeys, Predicate, Reduction, SortKey};
    use datafusion::datasource::provider_as_source;
    use datafusion::datasource::MemTable;
    use std::rc::Rc;

    const ADMITTED: [&str; 10] = [
        "Aggregate",
        "Concat",
        "Filter",
        "Join",
        "Limit",
        "Literal",
        "Project",
        "PromqlScalarBridge",
        "Scan",
        "Sort",
    ];

    const DEFERRED: [&str; 26] = [
        "Arithmetic",
        "BinaryOp",
        "BoolAnd",
        "BoolOr",
        "Case",
        "Cast",
        "Column",
        "Compare",
        "CurrentTimestamp",
        "Dedup",
        "EvalTimestamp",
        "FunctionCall",
        "InList",
        "IsNotNull",
        "IsNull",
        "Not",
        "PromqlInfoEnrich",
        "PromqlRelabel",
        "PromqlScalarFromVector",
        "PromqlSeriesSample",
        "PromqlSubquery",
        "PromqlVectorFromScalar",
        "SQLWindowFunc",
        "SetOp",
        "TimeRange",
        "TimeShift",
    ];

    fn sorted(names: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
        let mut held: Vec<&'static str> = names.into_iter().collect();
        held.sort_unstable();
        held
    }

    fn tables() -> TableSources {
        let schema = Arc::new(leaf_arrow_schema());
        let provider = MemTable::try_new(schema, vec![Vec::new()]).expect("an empty MemTable");
        TableSources::new().with_table(TABLE, provider_as_source(Arc::new(provider)))
    }

    #[test]
    fn the_two_tables_name_every_variant_of_the_tree_exactly_once() {
        let mut both = sorted(ADMITTED.into_iter().chain(DEFERRED));
        let before = both.len();
        both.dedup();
        assert_eq!(both.len(), before, "a name appears in both tables");
        assert_eq!(before, 36, "the tree has 36 variants");
        assert_eq!(
            both,
            sorted(every_query_expr().iter().map(|(name, _)| *name))
        );
    }

    #[test]
    fn every_name_in_the_two_tables_is_the_one_variant_name_gives_that_tree() {
        for (name, tree) in every_query_expr() {
            assert_eq!(crate::rows::variant_name(&tree), name);
        }
    }

    #[test]
    fn every_admitted_variant_lowers_to_a_logical_plan() {
        let tables = tables();
        for (name, tree) in every_query_expr() {
            if !ADMITTED.contains(&name) {
                continue;
            }
            lower(&tree, &tables).unwrap_or_else(|refusal| panic!("{name} is admitted: {refusal}"));
        }
    }

    #[test]
    fn every_deferred_variant_is_refused_by_name_with_one_of_the_four_reasons() {
        let tables = tables();
        for (name, tree) in every_query_expr() {
            if !DEFERRED.contains(&name) {
                continue;
            }
            let refusal = lower(&tree, &tables).expect_err("this step translates no such operator");
            assert_eq!(refusal.variant, format!("QueryExpr::{name}"));
            assert!(
                RefusalReason::TAGS.contains(&refusal.tag()),
                "{name}: {refusal}"
            );
        }
    }

    #[test]
    fn the_promql_only_nodes_are_refused_as_promql_only() {
        let tables = tables();
        for (name, tree) in every_query_expr() {
            let promql_only = [
                "PromqlVectorFromScalar",
                "PromqlScalarFromVector",
                "PromqlRelabel",
                "PromqlInfoEnrich",
                "PromqlSeriesSample",
            ];
            if !promql_only.contains(&name) {
                continue;
            }
            let refusal = lower(&tree, &tables).expect_err("refuses");
            assert_eq!(refusal.reason, RefusalReason::PromqlOnly, "{name}");
        }
    }

    #[test]
    fn the_time_axis_nodes_are_refused_for_want_of_an_evaluation_instant() {
        let tables = tables();
        for (name, tree) in every_query_expr() {
            let time_axis = [
                "EvalTimestamp",
                "CurrentTimestamp",
                "TimeRange",
                "TimeShift",
                "PromqlSubquery",
            ];
            if !time_axis.contains(&name) {
                continue;
            }
            let refusal = lower(&tree, &tables).expect_err("refuses");
            assert_eq!(refusal.reason, RefusalReason::TimeAxis, "{name}");
        }
    }

    #[test]
    fn a_time_series_leaf_is_refused_as_promql_only() {
        let tree = QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: "cpu_cores".to_owned(),
            },
            predicates: Vec::new(),
            schema: crate::df::variants::leaf_schema(),
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Scan");
        assert_eq!(refusal.reason, RefusalReason::PromqlOnly);
    }

    #[test]
    fn a_leaf_whose_bound_schema_disagrees_with_the_registered_table_is_refused() {
        let mut schema = crate::df::variants::leaf_schema();
        schema.columns[BYTES].dtype = asap_types::pre_asap::schema::DataType::Float64;
        let tree = QueryExpr::Scan {
            source: Source::Table {
                table_ref: TABLE.to_owned(),
            },
            predicates: Vec::new(),
            schema,
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Scan");
        assert_eq!(refusal.reason, RefusalReason::NoConstructor);
        assert!(refusal.detail.contains("bytes"), "{refusal}");
    }

    #[test]
    fn a_leaf_naming_no_registered_table_is_refused_with_the_names_that_are_registered() {
        let tree = QueryExpr::Scan {
            source: Source::Table {
                table_ref: "absent".to_owned(),
            },
            predicates: Vec::new(),
            schema: crate::df::variants::leaf_schema(),
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Scan");
        assert!(refusal.detail.contains(TABLE), "{refusal}");
    }

    #[test]
    fn a_partitioned_order_is_deferred_because_the_tree_does_not_carry_its_limit() {
        let tree = QueryExpr::Sort {
            keys: vec![SortKey {
                expr: QueryExpr::Column(LATENCY),
                ascending: false,
                nulls_first: false,
            }],
            partition_by: GroupKeys::by(vec![SERVICE]),
            child: scan(),
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Sort");
        assert_eq!(
            refusal.reason,
            RefusalReason::Deferred {
                issue: "sort-partition-limit-coupling".to_owned()
            }
        );
    }

    #[test]
    fn a_global_order_over_the_same_keys_is_admitted() {
        let tree = QueryExpr::Sort {
            keys: vec![SortKey {
                expr: QueryExpr::Column(LATENCY),
                ascending: false,
                nulls_first: false,
            }],
            partition_by: GroupKeys::none(),
            child: scan(),
        };
        lower(&tree, &tables()).expect("a global order translates");
    }

    #[test]
    fn a_grouping_by_every_column_except_these_is_refused_as_promql_only() {
        let tree = QueryExpr::Aggregate {
            reduction: Reduction::Reduce(GroupKeys::without(vec![SERVICE])),
            measures: vec![AggIntent::Sum { col: Some(BYTES) }],
            output_names: vec!["total".to_owned()],
            having: None,
            child: scan(),
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Aggregate");
        assert_eq!(refusal.reason, RefusalReason::PromqlOnly);
    }

    #[test]
    fn a_per_entity_reduction_is_refused_for_want_of_an_evaluation_instant() {
        let tree = QueryExpr::Aggregate {
            reduction: Reduction::PerEntity,
            measures: vec![AggIntent::Sum { col: Some(BYTES) }],
            output_names: vec!["total".to_owned()],
            having: None,
            child: scan(),
        };
        let refusal = lower(&tree, &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Aggregate");
        assert_eq!(refusal.reason, RefusalReason::TimeAxis);
    }

    #[test]
    fn a_having_clause_on_the_node_itself_becomes_a_filter_over_the_reduced_rows() {
        let tree = QueryExpr::Aggregate {
            reduction: Reduction::by(vec![SERVICE]),
            measures: vec![AggIntent::Sum { col: Some(BYTES) }],
            output_names: vec!["total".to_owned()],
            having: Some(Predicate(Rc::new(QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(1)),
                op: CompareOpKind::Gt,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Int64(10))),
            }))),
            child: scan(),
        };
        let plan = lower(&tree, &tables()).expect("a having clause translates");
        assert!(
            matches!(plan, LogicalPlan::Filter(_)),
            "the clause is the outermost node: {plan:?}"
        );
    }

    #[test]
    fn a_vector_matched_binary_operator_is_refused_as_promql_only() {
        let refusal = lower(&vector_matched_binary_op(), &tables()).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::BinaryOp");
        assert_eq!(refusal.reason, RefusalReason::PromqlOnly);
    }

    #[test]
    fn a_join_kind_outside_inner_and_left_is_deferred_by_kind() {
        for kind in [
            JoinKind::Right,
            JoinKind::Full,
            JoinKind::Cross,
            JoinKind::Semi,
            JoinKind::Anti,
        ] {
            let tree = QueryExpr::Join {
                kind: kind.clone(),
                pred: Predicate(Rc::new(QueryExpr::Compare {
                    left: Rc::new(QueryExpr::Column(SERVICE)),
                    op: CompareOpKind::Eq,
                    right: Rc::new(QueryExpr::Column(SERVICE + 4)),
                })),
                left: scan(),
                right: scan(),
            };
            let refusal = lower(&tree, &tables()).expect_err("refuses");
            assert_eq!(refusal.variant, "QueryExpr::Join");
            assert_eq!(
                refusal.reason,
                RefusalReason::Deferred {
                    issue: "join-kind".to_owned()
                },
                "{kind:?}"
            );
        }
    }

    #[test]
    fn the_measure_state_columns_are_projected_back_to_the_declared_output_names() {
        let tree = QueryExpr::Aggregate {
            reduction: Reduction::by(vec![SERVICE]),
            measures: vec![AggIntent::Sum { col: Some(BYTES) }],
            output_names: vec!["total".to_owned()],
            having: None,
            child: scan(),
        };
        let plan = lower(&tree, &tables()).expect("translates");
        let names: Vec<&str> = plan
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(names, vec!["service", "total"]);
        assert!(
            !plan
                .schema()
                .fields()
                .iter()
                .any(|field| field.name().starts_with(MEASURE_STATE_PREFIX)),
            "no internal state name reaches the node's output"
        );
    }
}
