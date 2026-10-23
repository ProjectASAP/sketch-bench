//! The workload's queries, as far as the harness needs them: each query's
//! text, its repeat interval, the longest range selector in it (which sets
//! when its first evaluation can see a full window), its accuracy
//! requirement, and for a quantile the query whose raw samples its rank
//! error is measured on.
//!
//! The harness does not evaluate PromQL: Prometheus is the reference, and
//! both systems' answers are compared series by series. A quantile's answer
//! is one interpolated value, which does not tell its rank, so for the two
//! quantile shapes the harness recognises it also asks Prometheus for the
//! samples the quantile is taken over ([`RankBasis`]).

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context};
use serde::Serialize;
use serde_json::Value;

use crate::data::label_key;

/// How a quantile's raw samples form its multisets, one per output series.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    /// One multiset per series: `quantile_over_time` over a range.
    PerSeries,
    /// One multiset per value of these labels: `quantile by (...)`, or
    /// `quantile (...)` with none.
    By(Vec<String>),
    /// One multiset per value of every label but these: `quantile without (...)`.
    Without(Vec<String>),
}

impl Grouping {
    /// The output series (its [`label_key`]) a raw series with `labels`
    /// belongs to.
    pub fn key(&self, labels: &BTreeMap<String, String>) -> String {
        let pairs =
            labels
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .filter(|(n, _)| match self {
                    Grouping::PerSeries => true,
                    Grouping::By(keep) => keep.iter().any(|k| k == n),
                    Grouping::Without(drop) => !drop.iter().any(|d| d == n),
                });
        label_key(pairs)
    }
}

/// Where a quantile's rank error is measured: `query`, evaluated by
/// Prometheus at the same time, returns the raw samples the quantile is taken
/// over, and `grouping` turns them into one multiset per output series.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankBasis {
    pub quantile: f64,
    pub query: String,
    pub grouping: Grouping,
}

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
    /// For a quantile, where its rank error is measured.
    pub rank_basis: Option<RankBasis>,
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

/// The index of the `)` closing the `(` at `open`, skipping quoted strings.
fn closing_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quoted = false;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `s` split at its first comma outside brackets and quotes.
fn split_first_arg(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0;
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' | '{' | '[' if !quoted => depth += 1,
            ')' | '}' | ']' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// A leading `by (a, b)` or `without (a)` clause of `s`, and what follows it.
fn grouping_clause(s: &str) -> Option<(Grouping, &str)> {
    let s = s.trim_start();
    let (word, rest) = s.split_at(s.find(|c: char| !c.is_ascii_alphabetic())?);
    let rest = rest.trim_start();
    if !matches!(word, "by" | "without") || !rest.starts_with('(') {
        return None;
    }
    let close = closing_paren(rest, 0)?;
    let labels: Vec<String> = rest[1..close]
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let grouping = if word == "by" {
        Grouping::By(labels)
    } else {
        Grouping::Without(labels)
    };
    Some((grouping, &rest[close + 1..]))
}

/// The rank basis of `quantile_over_time(q, X[W])` (its multisets are `X[W]`
/// per series) or of `quantile [by|without (...)] (q, E)` (`E` grouped);
/// `None` for anything else.
fn rank_basis(text: &str) -> Option<RankBasis> {
    let name_end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    let (name, rest) = text.split_at(name_end);
    let mut rest = rest.trim_start();
    let mut grouping = match name {
        "quantile_over_time" => Grouping::PerSeries,
        "quantile" => Grouping::By(Vec::new()),
        _ => return None,
    };
    if name == "quantile" {
        if let Some((g, after)) = grouping_clause(rest) {
            grouping = g;
            rest = after.trim_start();
        }
    }
    if !rest.starts_with('(') {
        return None;
    }
    let close = closing_paren(rest, 0)?;
    let (q, inner) = split_first_arg(&rest[1..close])?;
    let after = rest[close + 1..].trim();
    if !after.is_empty() {
        let (g, tail) = grouping_clause(after).filter(|_| name == "quantile")?;
        if !tail.trim().is_empty() {
            return None;
        }
        grouping = g;
    }
    let query = inner.trim();
    if name == "quantile_over_time" && !query.ends_with(']') {
        return None;
    }
    Some(RankBasis {
        quantile: q.trim().parse().ok()?,
        query: query.to_owned(),
        grouping,
    })
}

/// Every repeating query of `workload`, in order.
pub fn read_queries(workload: &Value) -> anyhow::Result<Vec<QuerySpec>> {
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
                rank_basis: rank_basis(&text),
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
        let q = read_queries(&w).unwrap();
        assert_eq!(q[0].id, "q01");
        assert_eq!(q[0].kind, "quantile_over_time");
        assert_eq!(q[0].rank_basis.as_ref().unwrap().query, "latency_ms[60m]");
        assert_eq!(q[0].accuracy_metric.as_deref(), Some("rank"));
        assert_eq!(q[1].kind, "topk");
        assert_eq!(q[1].range_ms, Some(300_000));
        assert_eq!(q[1].rank_basis, None);
        assert_eq!(q[2].kind, "quantile");
        assert_eq!(q[2].range_ms, None);
        assert_eq!(q[2].lead_ms(), 60_000);
        assert_eq!(
            q[2].rank_basis,
            Some(RankBasis {
                quantile: 0.99,
                query: "latency_ms".into(),
                grouping: Grouping::By(vec!["region".into()]),
            })
        );
        let one = workload_with(&w, &[&q[1]]);
        assert_eq!(
            one["query_workload"]["repeating_queries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rank_bases_of_both_quantile_shapes() {
        let b = |t: &str| rank_basis(t);
        assert_eq!(
            b("quantile_over_time(0.5, latency_ms{region=\"a,(b\"}[10m])").unwrap(),
            RankBasis {
                quantile: 0.5,
                query: "latency_ms{region=\"a,(b\"}[10m]".into(),
                grouping: Grouping::PerSeries,
            }
        );
        let trailing = b("quantile(0.9, rate(requests_total[5m])) without (instance)").unwrap();
        assert_eq!(trailing.query, "rate(requests_total[5m])");
        assert_eq!(
            trailing.grouping,
            Grouping::Without(vec!["instance".into()])
        );
        assert_eq!(
            b("quantile(0.9, latency_ms)").unwrap().grouping,
            Grouping::By(vec![])
        );
        assert_eq!(b("max_over_time(latency_ms[10m])"), None);
        assert_eq!(b("quantile_over_time(0.5, latency_ms)"), None);
        assert_eq!(b("quantile(0.9, latency_ms) + 1"), None);

        let labels: BTreeMap<String, String> = [
            ("__name__", "latency_ms"),
            ("instance", "instance-3"),
            ("region", "region-1"),
        ]
        .into_iter()
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(
            Grouping::PerSeries.key(&labels),
            "instance=instance-3,region=region-1"
        );
        assert_eq!(
            Grouping::By(vec!["region".into()]).key(&labels),
            "region=region-1"
        );
        assert_eq!(
            Grouping::Without(vec!["instance".into()]).key(&labels),
            "region=region-1"
        );
        assert_eq!(Grouping::By(vec![]).key(&labels), "");
    }
}
