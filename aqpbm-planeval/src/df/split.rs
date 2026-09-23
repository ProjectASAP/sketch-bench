use std::collections::{HashMap, HashSet};

use asap_types::post_asap::{
    ExecutableDag, ExecutableDagNode, ExecutableOperatorPayload, ExecutionTiming, PostAsapNodeId,
    SummarySchema,
};
use datafusion::arrow::datatypes::Schema as ArrowSchema;

use crate::df::refusal::Refusal;
use crate::df::schema::summary_schema_fields;
use crate::plan::Plan;
use crate::types::PlanId;

pub const STATE_TABLE_PREFIX: &str = "__asap_state_";

pub const PLAN_ID_DIGITS: usize = 8;

pub fn state_table_name(plan: &PlanId, node: PostAsapNodeId) -> String {
    let mut name = String::from(STATE_TABLE_PREFIX);
    for byte in plan.iter().take(PLAN_ID_DIGITS / 2) {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push('_');
    name.push_str(&node.0.to_string());
    name
}

#[derive(Debug, Clone)]
pub struct Cut {
    pub producer: PostAsapNodeId,
    pub table: String,
    pub intermediate_schema: SummarySchema,
    pub maintenance_nodes: Vec<PostAsapNodeId>,
}

impl Cut {
    pub fn state_table_schema(&self, produced: &ArrowSchema) -> Result<ArrowSchema, Refusal> {
        Ok(ArrowSchema::new(summary_schema_fields(
            &self.intermediate_schema,
            produced,
        )?))
    }
}

#[derive(Debug, Clone)]
pub struct DagSplit {
    pub cuts: Vec<Cut>,
    pub read_nodes: Vec<PostAsapNodeId>,
    pub no_summary_in_plan: bool,
}

impl DagSplit {
    pub fn is_an_advantage(&self) -> bool {
        !self.no_summary_in_plan
    }
}

pub fn split(plan: &Plan) -> Result<DagSplit, Refusal> {
    let dag = &plan.dag;
    let order = crate::df::post_asap::fold_order(dag)?;
    let nodes: HashMap<PostAsapNodeId, &ExecutableDagNode> =
        dag.nodes.iter().map(|node| (node.id, node)).collect();

    let reads_at_read_time = dag
        .nodes
        .iter()
        .any(|node| node.output_state.timing == ExecutionTiming::ReadTime);
    if !reads_at_read_time {
        return Ok(DagSplit {
            cuts: Vec::new(),
            read_nodes: order,
            no_summary_in_plan: true,
        });
    }

    let mut producers: Vec<PostAsapNodeId> = Vec::new();
    let mut schemas: HashMap<PostAsapNodeId, &SummarySchema> = HashMap::new();
    for edge in &dag.edges {
        if edge.data_state.timing != ExecutionTiming::MaintenanceTime {
            continue;
        }
        let consumer = *nodes.get(&edge.consumer).ok_or_else(|| {
            Refusal::no_constructor(
                "ExecutableDagEdge",
                format!("{:?} names no node", edge.consumer),
            )
        })?;
        if consumer.output_state.timing != ExecutionTiming::ReadTime {
            continue;
        }
        match schemas.get(&edge.producer) {
            Some(held) if **held != edge.intermediate_schema => {
                return Err(Refusal::no_constructor(
                    "ExecutableDagEdge",
                    format!(
                        "{:?} is cut by two edges that declare different intermediate schemas, so \
                         the state table has no one shape",
                        edge.producer
                    ),
                ))
            }
            Some(_) => {}
            None => {
                producers.push(edge.producer);
                schemas.insert(edge.producer, &edge.intermediate_schema);
            }
        }
    }

    let read_nodes: Vec<PostAsapNodeId> = {
        let stop: HashSet<PostAsapNodeId> = producers.iter().copied().collect();
        let reached = ancestors_of(dag, &[dag.root], &stop);
        order
            .iter()
            .copied()
            .filter(|id| reached.contains(id))
            .collect()
    };

    let mut cuts = Vec::with_capacity(producers.len());
    let mut below: HashSet<PostAsapNodeId> = HashSet::new();
    for producer in &producers {
        let reached = ancestors_of(dag, &[*producer], &HashSet::new());
        let maintenance_nodes: Vec<PostAsapNodeId> = order
            .iter()
            .copied()
            .filter(|id| reached.contains(id))
            .collect();
        below.extend(maintenance_nodes.iter().copied());
        cuts.push(Cut {
            producer: *producer,
            table: state_table_name(&plan.id, *producer),
            intermediate_schema: (*schemas
                .get(producer)
                .expect("every producer was inserted with its schema"))
            .clone(),
            maintenance_nodes,
        });
    }

    for id in &read_nodes {
        if below.contains(id) {
            continue;
        }
        let node = *nodes
            .get(id)
            .expect("every read node came out of the node map");
        if matches!(node.payload, ExecutableOperatorPayload::SummaryMerge) {
            return Err(Refusal::no_constructor(
                "ExecutableOperatorPayload::SummaryMerge",
                format!(
                    "{id:?} merges summary state above the cut, and the IR gives summary state no \
                     read-time data state to live in"
                ),
            ));
        }
    }

    Ok(DagSplit {
        cuts,
        read_nodes,
        no_summary_in_plan: false,
    })
}

fn ancestors_of(
    dag: &ExecutableDag,
    from: &[PostAsapNodeId],
    stop: &HashSet<PostAsapNodeId>,
) -> HashSet<PostAsapNodeId> {
    let mut producers: HashMap<PostAsapNodeId, Vec<PostAsapNodeId>> = HashMap::new();
    for edge in &dag.edges {
        producers
            .entry(edge.consumer)
            .or_default()
            .push(edge.producer);
    }

    let mut reached: HashSet<PostAsapNodeId> = HashSet::new();
    let mut pending: Vec<PostAsapNodeId> = from.to_vec();
    while let Some(id) = pending.pop() {
        if !reached.insert(id) {
            continue;
        }
        if stop.contains(&id) {
            continue;
        }
        for producer in producers.get(&id).into_iter().flatten() {
            pending.push(*producer);
        }
    }
    reached
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::df::variants::{kll_family, latency_update, plain_summary_schema, scan};
    use crate::plan::TimeRangeOrigin;
    use asap_types::post_asap::{
        EdgeRole, ExecutableDagEdge, ExecutionDataState, GroupingEdgeCompatibility,
        SummaryFamilyType, SummaryField, WindowEdgeCompatibility,
    };
    use asap_types::post_asap::{GroupingStrategy, SketchQuery, ValueOperation};
    use asap_types::pre_asap::schema::DataType;
    use asap_types::pre_asap::Reduction;

    fn sketch_state_schema(name: &str) -> SummarySchema {
        SummarySchema {
            fields: vec![SummaryField {
                name: name.to_owned(),
                dtype: kll_family(),
                nullable: false,
            }],
            time_index: None,
        }
    }

    fn node(
        id: u32,
        payload: ExecutableOperatorPayload,
        output_state: ExecutionDataState,
        output_schema: SummarySchema,
    ) -> ExecutableDagNode {
        ExecutableDagNode {
            id: PostAsapNodeId(id),
            payload,
            output_state,
            output_schema,
            guarantee: None,
        }
    }

    fn edge(producer: &ExecutableDagNode, consumer: &ExecutableDagNode) -> ExecutableDagEdge {
        ExecutableDagEdge {
            producer: producer.id,
            consumer: consumer.id,
            role: EdgeRole::Input,
            intermediate_schema: producer.output_schema.clone(),
            data_state: producer.output_state,
            grouping: GroupingEdgeCompatibility::NotApplicable,
            window: WindowEdgeCompatibility::NotApplicable,
        }
    }

    fn rows() -> SummarySchema {
        plain_summary_schema(&[("latency", DataType::Float64)])
    }

    fn leaf(id: u32) -> ExecutableDagNode {
        node(
            id,
            ExecutableOperatorPayload::Fallback {
                expression: (*scan()).clone(),
            },
            ExecutionDataState::MAINTENANCE_ROWS,
            rows(),
        )
    }

    fn summary_agg(id: u32, name: &str) -> ExecutableDagNode {
        node(
            id,
            ExecutableOperatorPayload::SummaryAgg {
                family: kll_family(),
                input: latency_update(),
                reduction: Reduction::by(vec![]),
                grouping: GroupingStrategy::PerSubpopulationInstance,
            },
            ExecutionDataState::MAINTENANCE_SUMMARY,
            sketch_state_schema(name),
        )
    }

    fn estimate(id: u32, name: &str) -> ExecutableDagNode {
        node(
            id,
            ExecutableOperatorPayload::SummaryEstimate {
                query: SketchQuery::Quantile { q: 0.5 },
            },
            ExecutionDataState::READ_ROWS,
            plain_summary_schema(&[(name, DataType::Float64)]),
        )
    }

    fn planned(nodes: Vec<ExecutableDagNode>, edges: Vec<ExecutableDagEdge>, root: u32) -> Plan {
        let dag = ExecutableDag {
            nodes,
            edges,
            root: PostAsapNodeId(root),
        };
        let order = crate::plan::topological_order(&dag).expect("the fixture is acyclic");
        Plan {
            id: [0x11; 32],
            dag,
            order,
            pre_asap: None,
            node_ids: None,
            time_range_origin: TimeRangeOrigin::Unknown,
        }
    }

    #[test]
    fn one_summary_under_one_readout_is_one_cut() {
        let source = leaf(0);
        let built = summary_agg(1, "held");
        let read = estimate(2, "held");
        let edges = vec![edge(&source, &built), edge(&built, &read)];
        let plan = planned(vec![source, built, read], edges, 2);

        let parts = split(&plan).expect("the plan splits");
        assert_eq!(parts.cuts.len(), 1);
        assert_eq!(parts.cuts[0].producer, PostAsapNodeId(1));
        assert_eq!(
            parts.cuts[0].maintenance_nodes,
            vec![PostAsapNodeId(0), PostAsapNodeId(1)],
            "the cut carries everything below it"
        );
        assert_eq!(
            parts.read_nodes,
            vec![PostAsapNodeId(1), PostAsapNodeId(2)],
            "the read query starts at the state table the cut producer becomes"
        );
        assert!(!parts.no_summary_in_plan);
    }

    #[test]
    fn two_summaries_under_one_root_are_two_cuts_and_two_state_tables() {
        let source = leaf(0);
        let left = summary_agg(1, "left");
        let right = summary_agg(2, "right");
        let read = node(
            3,
            ExecutableOperatorPayload::SummaryEstimate {
                query: SketchQuery::Quantile { q: 0.5 },
            },
            ExecutionDataState::READ_ROWS,
            plain_summary_schema(&[("left", DataType::Float64)]),
        );
        let edges = vec![
            edge(&source, &left),
            edge(&source, &right),
            edge(&left, &read),
            edge(&right, &read),
        ];
        let plan = planned(vec![source, left, right, read], edges, 3);

        let parts = split(&plan).expect("the plan splits");
        assert_eq!(parts.cuts.len(), 2, "one state table per cut producer");
        let tables: Vec<String> = parts.cuts.iter().map(|cut| cut.table.clone()).collect();
        assert_eq!(
            tables,
            vec![
                "__asap_state_11111111_1".to_owned(),
                "__asap_state_11111111_2".to_owned()
            ],
            "the two tables differ by the node they cut"
        );
        for cut in &parts.cuts {
            assert_eq!(cut.maintenance_nodes.len(), 2, "{:?}", cut.producer);
        }
        assert_eq!(
            parts.read_nodes,
            vec![PostAsapNodeId(1), PostAsapNodeId(2), PostAsapNodeId(3)]
        );
    }

    #[test]
    fn a_plan_with_no_read_time_node_is_all_read_query_and_no_advantage() {
        let source = leaf(0);
        let shaped = node(
            1,
            ExecutableOperatorPayload::Value {
                operation: ValueOperation::Limit { n: 5, offset: 0 },
                timing: ExecutionTiming::MaintenanceTime,
            },
            ExecutionDataState::MAINTENANCE_ROWS,
            rows(),
        );
        let edges = vec![edge(&source, &shaped)];
        let plan = planned(vec![source, shaped], edges, 1);

        let parts = split(&plan).expect("the plan splits");
        assert!(parts.cuts.is_empty(), "nothing is materialized");
        assert!(parts.no_summary_in_plan);
        assert!(
            !parts.is_an_advantage(),
            "arm B is arm A here and must not be counted as an advantage"
        );
        assert_eq!(
            parts.read_nodes,
            vec![PostAsapNodeId(0), PostAsapNodeId(1)],
            "the whole graph is the read query"
        );
    }

    #[test]
    fn a_merge_below_the_cut_belongs_to_the_maintenance_query() {
        let source = leaf(0);
        let one = summary_agg(1, "held");
        let two = summary_agg(2, "held");
        let merged = node(
            3,
            ExecutableOperatorPayload::SummaryMerge,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            sketch_state_schema("held"),
        );
        let read = estimate(4, "held");
        let edges = vec![
            edge(&source, &one),
            edge(&source, &two),
            edge(&one, &merged),
            edge(&two, &merged),
            edge(&merged, &read),
        ];
        let plan = planned(vec![source, one, two, merged, read], edges, 4);

        let parts = split(&plan).expect("the plan splits");
        assert_eq!(parts.cuts.len(), 1);
        assert_eq!(parts.cuts[0].producer, PostAsapNodeId(3));
        assert!(
            parts.cuts[0].maintenance_nodes.contains(&PostAsapNodeId(3)),
            "the merge runs at maintenance time"
        );
        assert_eq!(
            parts.read_nodes,
            vec![PostAsapNodeId(3), PostAsapNodeId(4)],
            "above the cut only the readout is left"
        );
    }

    #[test]
    fn a_merge_above_the_cut_is_refused_because_read_time_holds_no_summary_state() {
        let source = leaf(0);
        let built = summary_agg(1, "held");
        let read = estimate(2, "held");
        let merged = node(
            3,
            ExecutableOperatorPayload::SummaryMerge,
            ExecutionDataState::READ_ROWS,
            sketch_state_schema("held"),
        );
        let edges = vec![
            edge(&source, &built),
            edge(&built, &read),
            edge(&read, &merged),
        ];
        let plan = planned(vec![source, built, read, merged], edges, 3);

        let refused = split(&plan).unwrap_err();
        assert_eq!(
            refused.variant, "ExecutableOperatorPayload::SummaryMerge",
            "{refused}"
        );
        assert_eq!(refused.tag(), "no_constructor");
    }

    #[test]
    fn a_state_column_the_edge_declares_is_binary_and_a_group_key_keeps_its_own_type() {
        let source = leaf(0);
        let built = node(
            1,
            ExecutableOperatorPayload::SummaryAgg {
                family: kll_family(),
                input: latency_update(),
                reduction: Reduction::by(vec![0]),
                grouping: GroupingStrategy::PerSubpopulationInstance,
            },
            ExecutionDataState::MAINTENANCE_SUMMARY,
            SummarySchema {
                fields: vec![
                    SummaryField {
                        name: "service".to_owned(),
                        dtype: SummaryFamilyType::Plain(DataType::Utf8),
                        nullable: false,
                    },
                    SummaryField {
                        name: "held".to_owned(),
                        dtype: kll_family(),
                        nullable: false,
                    },
                ],
                time_index: None,
            },
        );
        let read = estimate(2, "held");
        let edges = vec![edge(&source, &built), edge(&built, &read)];
        let plan = planned(vec![source, built, read], edges, 2);

        let parts = split(&plan).expect("the plan splits");
        let produced = ArrowSchema::new(vec![
            datafusion::arrow::datatypes::Field::new(
                "service",
                datafusion::arrow::datatypes::DataType::Utf8,
                false,
            ),
            datafusion::arrow::datatypes::Field::new(
                "held",
                datafusion::arrow::datatypes::DataType::Binary,
                true,
            ),
        ]);
        let schema = parts.cuts[0]
            .state_table_schema(&produced)
            .expect("the edge types the state table");
        assert_eq!(
            schema.field(0).data_type(),
            &datafusion::arrow::datatypes::DataType::Utf8
        );
        assert_eq!(
            schema.field(1).data_type(),
            &datafusion::arrow::datatypes::DataType::Binary
        );
        assert!(
            schema.field(1).is_nullable(),
            "the state table follows what the maintenance query actually produced"
        );
    }

    #[test]
    fn a_state_table_is_named_after_the_plan_and_the_node_it_cuts() {
        let mut id: PlanId = [0; 32];
        id[0] = 0xde;
        id[1] = 0xad;
        id[2] = 0xbe;
        id[3] = 0xef;
        id[4] = 0xff;
        assert_eq!(
            state_table_name(&id, PostAsapNodeId(7)),
            "__asap_state_deadbeef_7"
        );
    }
}
