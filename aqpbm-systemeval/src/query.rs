use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use aqpbm_usecase::accuracy::{AnswerAt, Labels};
use aqpbm_usecase::data::Series;
use aqpbm_usecase::score::score;
use aqpbm_usecase::spec::QuerySpec;
use serde_json::{json, Value};

const KEPT_FAILURES: usize = 5;

pub struct Asked {
    pub seconds: f64,
    pub outcome: Result<Vec<(Labels, f64)>, String>,
}

fn seconds_text(ms: i64) -> String {
    format!("{}.{:03}", ms.div_euclid(1000), ms.rem_euclid(1000))
}

fn vector(body: &Value) -> Result<Vec<(Labels, f64)>> {
    if body["status"] != "success" {
        bail!("{}", body["error"].as_str().unwrap_or("query failed"));
    }
    if body["data"]["resultType"] != "vector" {
        bail!("result type {} is not a vector", body["data"]["resultType"]);
    }
    let mut out = Vec::new();
    for sample in body["data"]["result"].as_array().into_iter().flatten() {
        let labels: Labels = serde_json::from_value(sample["metric"].clone())?;
        let text = sample["value"][1]
            .as_str()
            .context("a sample has no value")?;
        out.push((labels, text.parse::<f64>()?));
    }
    Ok(out)
}

pub fn ask(agent: &ureq::Agent, base: &str, promql: &str, time_ms: i64) -> Asked {
    let started = Instant::now();
    let response = agent
        .get(&format!("{base}/api/v1/query"))
        .query("query", promql)
        .query("time", &seconds_text(time_ms))
        .call();
    let text = match response {
        Ok(response) => response.into_string().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(_, response)) => response.into_string().map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let seconds = started.elapsed().as_secs_f64();
    let outcome = text.and_then(|text| {
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("{e}: {text}"))
            .and_then(|body| vector(&body).map_err(|e| e.to_string()))
    });
    Asked { seconds, outcome }
}

#[derive(Default)]
pub struct Collected {
    answered: Vec<AnswerAt>,
    costs: Vec<Value>,
    failures: Vec<Value>,
    failed: usize,
    not_finite: usize,
    times: Vec<i64>,
}

impl Collected {
    pub fn record(&mut self, t: i64, asked: Asked) {
        self.times.push(t);
        let returned = match asked.outcome {
            Ok(samples) => {
                let count = samples.len();
                for (labels, value) in samples {
                    if value.is_finite() {
                        self.answered.push((t, labels, value));
                    } else {
                        self.not_finite += 1;
                    }
                }
                Some(count)
            }
            Err(reason) => {
                self.failed += 1;
                if self.failures.len() < KEPT_FAILURES {
                    self.failures.push(json!({"time": t, "error": reason}));
                }
                None
            }
        };
        self.costs
            .push(json!({"time": t, "query_seconds": asked.seconds, "series_returned": returned}));
    }

    pub fn finish(
        self,
        query: &QuerySpec,
        series: &[Series],
        exact: Option<&BTreeMap<String, Vec<AnswerAt>>>,
    ) -> Result<Value> {
        let mut out = json!({
            "id": query.id,
            "promql": query.promql,
            "unit": query.unit,
            "threshold": query.threshold,
            "evaluations": self.times.len(),
            "failed_evaluations": self.failed,
            "failures": self.failures,
            "not_finite_answers": self.not_finite,
            "costs": self.costs,
        });
        if self.failed == self.times.len() {
            out["unsupported"] = self
                .failures
                .first()
                .map_or(json!("no evaluation answered"), |f| f["error"].clone());
            return Ok(out);
        }
        let scored = score(query, series, &self.times, &self.answered, exact)
            .with_context(|| format!("scoring {}", query.id))?;
        out["answers"] = json!(scored.answers);
        out["computed_truth"] = json!(scored.computed_truth);
        Ok(out)
    }
}
