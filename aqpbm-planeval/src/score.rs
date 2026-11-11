use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use aqpbm_core::accuracy::scalar_error;
use serde_json::{json, Value};

use crate::accuracy::{keyed, without_name, AnswerAt, Labels, QueryTruth, Scores};
use crate::data::Series;
use crate::plan::{workload, Arm};
use crate::spec::{QuerySpec, Spec};

pub fn read_answers(path: &Path) -> Result<BTreeMap<String, Vec<AnswerAt>>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let document: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut out = BTreeMap::new();
    for query in document["queries"]
        .as_array()
        .with_context(|| format!("{} has no queries", path.display()))?
    {
        let id = query["id"]
            .as_str()
            .with_context(|| format!("a query in {} has no id", path.display()))?;
        let mut answers = Vec::new();
        for answer in query["answers"].as_array().into_iter().flatten() {
            let (Some(time), Some(value)) = (answer["time"].as_i64(), answer["value"].as_f64())
            else {
                continue;
            };
            let labels: Labels = serde_json::from_value(answer["series"].clone())?;
            answers.push((time, labels, value));
        }
        out.insert(id.to_string(), answers);
    }
    Ok(out)
}

pub struct Scored {
    pub answers: Vec<Value>,
    pub computed_truth: Option<Vec<Value>>,
}

pub fn score(
    spec: &Spec,
    query: &QuerySpec,
    arm: &Arm,
    series: &[Series],
    times: &[i64],
    answered: &[AnswerAt],
    exact: Option<&BTreeMap<String, Vec<AnswerAt>>>,
) -> Result<Scored> {
    let truth = QueryTruth::of(&workload(spec, query, arm)?)?;
    let computed_truth = truth
        .as_ref()
        .map(|truth| truth.score(series, times, answered))
        .transpose()
        .with_context(|| format!("scoring {} against its computed truth", query.id))?;
    let reference = exact
        .map(|exact| keyed(exact.get(&query.id).map_or(&[][..], |a| a.as_slice())));
    let rank_errors: HashMap<(i64, Labels), Scores> = match (&truth, &reference) {
        (Some(truth), Some(reference)) => truth
            .reference_rank_errors(series, times, answered, reference)
            .with_context(|| format!("scoring {} against the exact plan", query.id))?
            .into_iter()
            .filter_map(|(t, labels, scores)| Some(((t, labels), scores?)))
            .collect(),
        _ => HashMap::new(),
    };
    let answers = answered
        .iter()
        .map(|(t, labels, value)| {
            let mut answer = json!({"time": t, "series": labels, "value": value});
            let key = (*t, without_name(labels));
            if let Some(&exact_value) = reference.as_ref().and_then(|r| r.get(&key)) {
                let mut errors = scalar_error(exact_value, &[*value]);
                if let Some(rank_error) = rank_errors.get(&key).and_then(|s| s.get("rank_error")) {
                    errors.insert("rank_error".into(), *rank_error);
                }
                answer["exact_plan"] = json!(errors);
            }
            answer
        })
        .collect();
    let computed_truth = computed_truth.map(|cells| {
        cells
            .into_iter()
            .map(|(t, labels, scores)| json!({"time": t, "series": labels, "scores": scores}))
            .collect()
    });
    Ok(Scored {
        answers,
        computed_truth,
    })
}
