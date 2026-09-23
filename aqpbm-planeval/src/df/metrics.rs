use std::collections::BTreeMap;

use asap_types::post_asap::PostAsapNodeId;
use datafusion::physical_plan::ExecutionPlan;

pub const NODE_ALIAS_PREFIX: &str = "__asap_n";

pub fn node_alias(node: PostAsapNodeId, field: &str) -> String {
    format!("{NODE_ALIAS_PREFIX}{}_{field}", node.0)
}

pub fn node_of_alias(name: &str) -> Option<PostAsapNodeId> {
    split_node_alias(name).map(|(node, _)| node)
}

pub fn strip_node_alias(name: &str) -> &str {
    match split_node_alias(name) {
        Some((_, field)) => field,
        None => name,
    }
}

fn split_node_alias(name: &str) -> Option<(PostAsapNodeId, &str)> {
    let rest = name.strip_prefix(NODE_ALIAS_PREFIX)?;
    let (digits, field) = rest.split_once('_')?;
    let node = digits.parse().ok().map(PostAsapNodeId)?;
    Some((node, field))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ElapsedCompute {
    pub per_node: BTreeMap<PostAsapNodeId, u64>,
    pub engine_overhead_ns: u64,
    pub attributed_operators: usize,
    pub unattributed_operators: usize,
}

impl ElapsedCompute {
    pub fn of_plan(plan: &dyn ExecutionPlan) -> Self {
        let mut held = Self::default();
        held.add_plan(plan);
        held
    }

    pub fn add_plan(&mut self, plan: &dyn ExecutionPlan) {
        let elapsed_ns = plan
            .metrics()
            .and_then(|set| set.elapsed_compute())
            .unwrap_or(0) as u64;
        match owning_node(plan) {
            Some(node) => {
                *self.per_node.entry(node).or_insert(0) += elapsed_ns;
                self.attributed_operators += 1;
            }
            None => {
                self.engine_overhead_ns += elapsed_ns;
                self.unattributed_operators += 1;
            }
        }
        for child in plan.children() {
            self.add_plan(child.as_ref());
        }
    }

    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            per_node: self
                .per_node
                .iter()
                .map(|(node, elapsed_ns)| {
                    let before = earlier.per_node.get(node).copied().unwrap_or(0);
                    (*node, elapsed_ns.saturating_sub(before))
                })
                .collect(),
            engine_overhead_ns: self
                .engine_overhead_ns
                .saturating_sub(earlier.engine_overhead_ns),
            attributed_operators: self.attributed_operators,
            unattributed_operators: self.unattributed_operators,
        }
    }

    pub fn attributed_ns(&self) -> u64 {
        self.per_node.values().sum()
    }

    pub fn operators(&self) -> usize {
        self.attributed_operators + self.unattributed_operators
    }
}

fn owning_node(plan: &dyn ExecutionPlan) -> Option<PostAsapNodeId> {
    plan.schema()
        .fields()
        .iter()
        .find_map(|field| node_of_alias(field.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_round_trips_through_the_node_it_names() {
        let alias = node_alias(PostAsapNodeId(7), "latency_state");
        assert_eq!(alias, "__asap_n7_latency_state");
        assert_eq!(node_of_alias(&alias), Some(PostAsapNodeId(7)));
    }

    #[test]
    fn a_name_the_translator_did_not_alias_names_no_node() {
        for name in [
            "latency",
            "__asap_state_0011aabb_2",
            "__asap_n_x",
            "__asap_nx_1",
        ] {
            assert_eq!(node_of_alias(name), None, "{name}");
        }
    }

    #[test]
    fn a_field_name_holding_underscores_still_names_its_node() {
        let alias = node_alias(PostAsapNodeId(12), "a_b_c");
        assert_eq!(node_of_alias(&alias), Some(PostAsapNodeId(12)));
        assert_eq!(strip_node_alias(&alias), "a_b_c");
    }

    #[test]
    fn stripping_leaves_a_name_the_translator_did_not_alias_alone() {
        assert_eq!(strip_node_alias("latency"), "latency");
        assert_eq!(
            strip_node_alias("__asap_state_0011aabb_2"),
            "__asap_state_0011aabb_2"
        );
    }
}
