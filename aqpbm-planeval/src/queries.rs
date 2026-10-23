//! The workload's queries, as far as the harness needs them: each query's
//! text, its repeat interval, the longest range selector in it (which sets
//! when its first evaluation can see a full window), its accuracy
//! requirement, and whether its rank error can be checked against the
//! generated samples.
//!
//! The harness does not interpret PromQL: both systems evaluate the query,
//! and their answers are compared series by series. Only
//! `quantile_over_time(q, <gauge>[W])` is recognised, so its rank error is
//! measured too.

use anyhow::{bail, ensure, Context};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct QuerySpec {
    /// `q01`, `q02`, ... in workload order.
    pub id: String,
    pub text: String,
    /// The outermost function or aggregation, e.g. `topk`, `sum`, `rate`.
    pub kind: String,
    pub interval_ms: i64,
    /// The longest range selector, if the query has one.
    pub range_ms: Option<i64>,
    /// The requirement's `accuracy_metric` and `max_error`, if any.
    pub accuracy_metric: Option<String>,
    pub max_error: Option<f64>,
    /// `(q, W)` of `quantile_over_time(q, <gauge>[W])` over the generated
    /// gauge: its answers' rank error is measured on the window's samples.
    pub rank_check: Option<(f64, i64)>,
    /// The query's entry in the workload, as written.
    #[serde(skip)]
    pub entry: Value,
}

impl QuerySpec {
    /// How long after the stream's start the first evaluation sees a full
    /// window: the longest range, or one interval for a query without one.
    pub fn lead_ms(&self) -> i64 {
        self.range_ms.unwrap_or(self.interval_ms)
    }
}

pub fn parse_duration_ms(s: &str) -> anyhow::Result<i64> {
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = num.parse().with_context(|| format!("duration {s:?}"))?;
    Ok(n * match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => bail!("duration {s:?}: unsupported unit"),
    })
}

/// The longest `[...]` range in `text`.
fn longest_range(text: &str) -> anyhow::Result<Option<i64>> {
    let mut longest = None;
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let close = rest[open..]
            .find(']')
            .with_context(|| format!("unclosed range in {text:?}"))?;
        let range = parse_duration_ms(rest[open + 1..open + close].trim())?;
        longest = longest.max(Some(range));
        rest = &rest[open + close + 1..];
    }
    Ok(longest)
}

/// `(q, W)` if `text` is `quantile_over_time(q, gauge[W])`.
fn quantile_over_gauge(text: &str, gauge: &str) -> Option<(f64, i64)> {
    let inner = text
        .strip_prefix("quantile_over_time(")?
        .strip_suffix("])")?;
    let (q, selector) = inner.split_once(',')?;
    let (metric, window) = selector.trim().split_once('[')?;
    (metric == gauge).then_some(())?;
    Some((q.trim().parse().ok()?, parse_duration_ms(window).ok()?))
}

/// Every repeating query of `workload`, in order.
pub fn read_queries(workload: &Value, gauge: &str) -> anyhow::Result<Vec<QuerySpec>> {
    let entries = workload
        .pointer("/query_workload/repeating_queries")
        .and_then(Value::as_array)
        .context("workload has no query_workload.repeating_queries")?;
    ensure!(!entries.is_empty(), "the workload has no query");
    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let text = entry["query"]
                .as_str()
                .context("query text")?
                .trim()
                .to_owned();
            let interval_ms = entry
                .pointer("/demand/fixed_interval")
                .and_then(Value::as_i64)
                .with_context(|| format!("{text:?}: only demand.fixed_interval is supported"))?;
            ensure!(interval_ms > 0, "{text:?}: interval must be positive");
            let accuracy = entry.pointer("/requirements/accuracy");
            Ok(QuerySpec {
                id: format!("q{:02}", i + 1),
                kind: text
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                interval_ms,
                range_ms: longest_range(&text)?,
                accuracy_metric: accuracy
                    .and_then(|a| a["accuracy_metric"].as_str())
                    .map(str::to_owned),
                max_error: accuracy.and_then(|a| a["max_error"].as_f64()),
                rank_check: quantile_over_gauge(&text, gauge),
                entry: entry.clone(),
                text,
            })
        })
        .collect()
}

/// `workload` with only `queries` as its repeating queries.
pub fn workload_with(workload: &Value, queries: &[&QuerySpec]) -> Value {
    let mut w = workload.clone();
    w["query_workload"]["repeating_queries"] =
        Value::Array(queries.iter().map(|q| q.entry.clone()).collect());
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queries_are_read_in_order_with_their_ranges() {
        let w = json!({"query_workload": {"repeating_queries": [
            {"query": "quantile_over_time(0.99, latency_ms[60m])",
             "demand": {"fixed_interval": 600000},
             "requirements": {"accuracy": {"accuracy_metric": "rank", "max_error": 0.01}}},
            {"query": "topk(3, sum by (service) (rate(requests_total[5m])))",
             "demand": {"fixed_interval": 60000}},
            {"query": "quantile by (region) (0.99, latency_ms)",
             "demand": {"fixed_interval": 60000}},
        ]}});
        let q = read_queries(&w, "latency_ms").unwrap();
        assert_eq!(q[0].id, "q01");
        assert_eq!(q[0].kind, "quantile_over_time");
        assert_eq!(q[0].rank_check, Some((0.99, 3_600_000)));
        assert_eq!(q[0].accuracy_metric.as_deref(), Some("rank"));
        assert_eq!(q[1].kind, "topk");
        assert_eq!(q[1].range_ms, Some(300_000));
        assert_eq!(q[1].rank_check, None);
        assert_eq!(q[2].kind, "quantile");
        assert_eq!(q[2].range_ms, None);
        assert_eq!(q[2].lead_ms(), 60_000);
        let one = workload_with(&w, &[&q[1]]);
        assert_eq!(
            one["query_workload"]["repeating_queries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
