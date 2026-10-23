//! What a plan does for each of its queries, read from `plan.json`: the
//! summaries under the query's root (a sketch, or an exact aggregate, and
//! whether it is built at ingestion or at query time), and the accuracy
//! guarantee the planner states for the root.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize)]
pub struct QueryMethod {
    /// The summaries under the root, e.g. `Kll(k=361)@ingestion`, joined by
    /// ` + `; `raw@query` when the plan reads raw samples only.
    pub method: String,
    /// True if any summary is a sketch.
    pub sketch: bool,
    /// The root's guarantee: `exact`, `rank<=0.0099 (p_fail 0.000625)`, ...
    pub guarantee: String,
    /// The query is in `selection.queries_accepted_without_guarantee`.
    pub accepted_without_guarantee: bool,
}

fn timing(node: &Value) -> &str {
    match node.pointer("/output_state/timing").and_then(Value::as_str) {
        Some("ingestion_time") => "ingestion",
        Some("query_time") => "query",
        Some(other) => other,
        None => "?",
    }
}

/// `{"Sketch": [{"algorithm": "Kll", "params": {"Kll": {"k": 361, ...}}}, ...]}`
/// -> `Kll(k=361)`; `{"ExactAggregate": ["Rate", "Rate"]}` -> `Exact(Rate)`.
fn family_name(family: &Value) -> (String, bool) {
    if let Some(sketch) = family.pointer("/Sketch/0") {
        let algorithm = sketch["algorithm"].as_str().unwrap_or("?");
        let params = sketch
            .pointer(&format!("/params/{algorithm}"))
            .and_then(Value::as_object)
            .map(|p| {
                p.iter()
                    .filter(|(k, _)| k.as_str() != "failure_probability")
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        return (format!("{algorithm}({params})"), true);
    }
    if let Some(exact) = family.pointer("/ExactAggregate/0").and_then(Value::as_str) {
        return (format!("Exact({exact})"), false);
    }
    let name = family
        .as_object()
        .and_then(|o| o.keys().next().cloned())
        .unwrap_or_else(|| family.to_string());
    (name, false)
}

fn guarantee_text(node: &Value) -> String {
    let Some(g) = node.get("guarantee").filter(|g| !g.is_null()) else {
        return "none".into();
    };
    let provenance: Vec<&str> = g["provenance"]
        .as_array()
        .map(|p| p.iter().filter_map(|x| x["kind"].as_str()).collect())
        .unwrap_or_default();
    let spec = &g["spec"];
    if spec.pointer("/error/op").and_then(Value::as_str) == Some("zero")
        && provenance.iter().all(|k| *k == "exact")
    {
        return "exact".into();
    }
    let metric = spec["accuracy_metric"].as_str().unwrap_or("?");
    let error = spec
        .pointer("/error/value")
        .and_then(Value::as_f64)
        .map_or_else(|| spec["error"].to_string(), |e| format!("{e:.4}"));
    let p_fail = spec
        .pointer("/failure_probability/value")
        .and_then(Value::as_f64)
        .map(|p| format!(" (p_fail {p})"))
        .unwrap_or_default();
    format!("{metric}<={error}{p_fail}")
}

/// Query text -> what the plan does for it.
pub fn query_methods(plan: &Value) -> BTreeMap<String, QueryMethod> {
    let dag = &plan["dag"]["dag"];
    let nodes: BTreeMap<i64, &Value> = dag["nodes"]
        .as_array()
        .map(|n| {
            n.iter()
                .filter_map(|n| Some((n["id"].as_i64()?, n)))
                .collect()
        })
        .unwrap_or_default();
    let mut producers: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for edge in dag["edges"].as_array().into_iter().flatten() {
        if let (Some(p), Some(c)) = (edge["producer"].as_i64(), edge["consumer"].as_i64()) {
            producers.entry(c).or_default().push(p);
        }
    }
    let unguaranteed: BTreeSet<&str> = plan
        .pointer("/selection/queries_accepted_without_guarantee")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut out = BTreeMap::new();
    for q in plan["queries"].as_array().into_iter().flatten() {
        let (Some(text), Some(root)) = (q["query"].as_str(), q["root_node_id"].as_i64()) else {
            continue;
        };
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        let mut summaries = BTreeSet::new();
        let mut sketch = false;
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(node) = nodes.get(&id) {
                if let Some(family) = node.pointer("/payload/ASAP/SummaryAgg/family") {
                    let (name, is_sketch) = family_name(family);
                    sketch |= is_sketch;
                    summaries.insert(format!("{name}@{}", timing(node)));
                }
            }
            stack.extend(producers.get(&id).into_iter().flatten());
        }
        out.insert(
            text.to_owned(),
            QueryMethod {
                method: if summaries.is_empty() {
                    "raw@query".into()
                } else {
                    summaries.into_iter().collect::<Vec<_>>().join(" + ")
                },
                sketch,
                guarantee: nodes.get(&root).map_or("?".into(), |n| guarantee_text(n)),
                accepted_without_guarantee: unguaranteed.contains(text),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn methods_follow_edges_from_each_root() {
        let plan = json!({
            "queries": [{"query": "q", "root_node_id": 2}, {"query": "r", "root_node_id": 3}],
            "selection": {"queries_accepted_without_guarantee": ["r"]},
            "dag": {"dag": {
                "nodes": [
                    {"id": 0, "payload": {"NonASAP": {"Scan": {}}}},
                    {"id": 1, "output_state": {"timing": "ingestion_time"},
                     "payload": {"ASAP": {"SummaryAgg": {"family": {"Sketch": [
                        {"algorithm": "Kll", "params": {"Kll": {"k": 361, "failure_probability": 0.1}}},
                        "PerSubpopulationInstance"]}}}}},
                    {"id": 2, "payload": {"NonASAP": {"Project": {}}},
                     "guarantee": {"spec": {"accuracy_metric": "rank",
                        "error": {"op": "constant", "value": 0.0099},
                        "failure_probability": {"op": "constant", "value": 0.000625}},
                        "provenance": [{"kind": "sketch_evaluation"}]}},
                    {"id": 3, "payload": {"NonASAP": {"Project": {}}},
                     "guarantee": {"spec": {"accuracy_metric": "absolute", "error": {"op": "zero"}},
                        "provenance": [{"kind": "exact"}]}},
                ],
                "edges": [{"producer": 0, "consumer": 1}, {"producer": 1, "consumer": 2},
                          {"producer": 0, "consumer": 3}],
            }},
        });
        let m = query_methods(&plan);
        assert_eq!(m["q"].method, "Kll(k=361)@ingestion");
        assert!(m["q"].sketch);
        assert_eq!(m["q"].guarantee, "rank<=0.0099 (p_fail 0.000625)");
        assert_eq!(m["r"].method, "raw@query");
        assert_eq!(m["r"].guarantee, "exact");
        assert!(m["r"].accepted_without_guarantee);
    }
}
