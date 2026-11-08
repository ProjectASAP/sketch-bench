use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{anyhow, Result};
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::keyed::KeyedEntropyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, ReferenceRankErrorGT};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::accuracy::windowed::{WindowScore, Windowed};
use aqpbm_core::accuracy::GroundTruth;
use aqpbm_datagen::{ColumnData, GeneratedTable};
use promql_parser::label::MatchOp;
use promql_parser::parser::{self, Expr, LabelModifier, MatrixSelector};

use crate::data::Series;

pub type Labels = BTreeMap<String, String>;

pub type Scores = BTreeMap<String, f64>;

pub type Cell = (i64, Labels, Option<Scores>);

pub type AnswerAt = (i64, Labels, f64);

const TIME_COLUMN: usize = 0;

enum Statistic {
    Quantile(f64),
    Entropy,
    Distinct,
    Topk { k: usize, by: String },
}

pub struct QueryTruth {
    statistic: Statistic,
    metric: Option<String>,
    matchers: Vec<(String, String)>,
    width_ms: i64,
}

struct Samples {
    table: GeneratedTable,
    label_names: Vec<String>,
}

impl Samples {
    fn value_column(&self) -> usize {
        self.label_names.len() + 1
    }

    fn weight_column(&self) -> usize {
        self.label_names.len() + 2
    }

    fn label_columns(&self) -> Vec<usize> {
        (1..=self.label_names.len()).collect()
    }

    fn labels_of(&self, group: &[String]) -> Labels {
        self.label_names
            .iter()
            .zip(group)
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

pub fn without_name(labels: &Labels) -> Labels {
    labels
        .iter()
        .filter(|(name, _)| name.as_str() != "__name__")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub fn keyed(answers: &[AnswerAt]) -> HashMap<(i64, Labels), f64> {
    answers
        .iter()
        .map(|(time, labels, value)| ((*time, without_name(labels)), *value))
        .collect()
}

fn over_time(expr: &Expr) -> Option<(&'static str, &[Box<Expr>], &MatrixSelector)> {
    let Expr::Call(call) = expr else {
        return None;
    };
    let Some(Expr::MatrixSelector(range)) = call.args.args.last().map(|arg| arg.as_ref()) else {
        return None;
    };
    Some((call.func.name, &call.args.args, range))
}

fn number(expr: Option<&Expr>) -> Option<f64> {
    match expr {
        Some(Expr::NumberLiteral(n)) => Some(n.val),
        _ => None,
    }
}

impl QueryTruth {
    pub fn of(promql: &str) -> Result<Option<QueryTruth>> {
        let expr = parser::parse(promql).map_err(|e| anyhow!("parsing {promql}: {e}"))?;
        if let Expr::Aggregate(top) = &expr {
            if top.op.to_string() != "topk" {
                return Ok(None);
            }
            let Some(k) = number(top.param.as_deref()) else {
                return Ok(None);
            };
            let Expr::Aggregate(sum) = top.expr.as_ref() else {
                return Ok(None);
            };
            let Some(LabelModifier::Include(by)) = &sum.modifier else {
                return Ok(None);
            };
            if sum.op.to_string() != "sum" || by.labels.len() != 1 {
                return Ok(None);
            }
            let Some(("count_over_time", _, range)) = over_time(&sum.expr) else {
                return Ok(None);
            };
            let statistic = Statistic::Topk {
                k: k as usize,
                by: by.labels[0].clone(),
            };
            return Ok(QueryTruth::over(statistic, range));
        }
        Ok(match over_time(&expr) {
            Some(("quantile_over_time", args, range)) => number(args.first().map(|a| a.as_ref()))
                .and_then(|q| QueryTruth::over(Statistic::Quantile(q), range)),
            Some(("entropy_over_time", _, range)) => QueryTruth::over(Statistic::Entropy, range),
            Some(("distinct_over_time", _, range)) => QueryTruth::over(Statistic::Distinct, range),
            _ => None,
        })
    }

    fn over(statistic: Statistic, range: &MatrixSelector) -> Option<QueryTruth> {
        let selector = &range.vs;
        let plain = selector.offset.is_none()
            && selector.at.is_none()
            && selector.matchers.or_matchers.is_empty()
            && selector
                .matchers
                .matchers
                .iter()
                .all(|m| matches!(m.op, MatchOp::Equal));
        plain.then(|| QueryTruth {
            statistic,
            metric: selector.name.clone(),
            matchers: selector
                .matchers
                .matchers
                .iter()
                .map(|m| (m.name.clone(), m.value.clone()))
                .collect(),
            width_ms: range.range.as_millis() as i64,
        })
    }

    fn selects(&self, labels: &Labels) -> bool {
        self.metric
            .as_ref()
            .is_none_or(|metric| labels.get("__name__") == Some(metric))
            && self
                .matchers
                .iter()
                .all(|(name, value)| labels.get(name).map_or("", String::as_str) == value)
    }

    fn samples(&self, series: &[Series]) -> Samples {
        let chosen: Vec<&Series> = series.iter().filter(|s| self.selects(&s.labels)).collect();
        let label_names: Vec<String> = chosen
            .iter()
            .flat_map(|s| s.labels.keys())
            .filter(|name| name.as_str() != "__name__")
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let rows: usize = chosen.iter().map(|s| s.points.len()).sum();
        let mut time = Vec::with_capacity(rows);
        let mut value = Vec::with_capacity(rows);
        let mut labels: Vec<Vec<String>> = vec![Vec::with_capacity(rows); label_names.len()];
        for s in chosen {
            for &(t, v) in &s.points {
                time.push(t);
                value.push(v);
                for (column, name) in labels.iter_mut().zip(&label_names) {
                    column.push(s.labels.get(name).cloned().unwrap_or_default());
                }
            }
        }
        let mut column_title = vec!["time".to_string()];
        column_title.extend(label_names.iter().cloned());
        column_title.extend(["value".to_string(), "weight".to_string()]);
        let mut data = vec![ColumnData::Int64(time)];
        data.extend(labels.into_iter().map(ColumnData::String));
        data.push(ColumnData::Float64(value));
        data.push(ColumnData::Int64(vec![1; rows]));
        Samples {
            table: GeneratedTable {
                column_num: data.len() as u32,
                column_title,
                row_num: rows as u64,
                data,
            },
            label_names,
        }
    }

    fn windowed<G>(&self, ground_truth: G, group_columns: Vec<usize>, ends: &[i64]) -> Windowed<G> {
        Windowed {
            ground_truth,
            time_column: TIME_COLUMN,
            group_columns,
            width: self.width_ms,
            ends: ends.to_vec(),
        }
    }

    fn per_series<G, F>(
        &self,
        samples: &Samples,
        ground_truth: G,
        ends: &[i64],
        mut answer: F,
    ) -> Result<Vec<Cell>>
    where
        G: GroundTruth,
        F: FnMut(i64, &Labels) -> Option<G::Answer>,
        G::Answer: Clone,
    {
        let cells = self
            .windowed(ground_truth, samples.label_columns(), ends)
            .score(&samples.table, |end, group, probes| {
                answer(end, &samples.labels_of(group)).map(|a| vec![a; probes.len()])
            })?;
        Ok(cells_of(samples, cells))
    }

    pub fn score(
        &self,
        series: &[Series],
        ends: &[i64],
        answers: &[AnswerAt],
    ) -> Result<Vec<Cell>> {
        let samples = self.samples(series);
        let value = samples.value_column();
        let by_key = keyed(answers);
        let lookup = |end: i64, labels: &Labels| by_key.get(&(end, labels.clone())).copied();
        match &self.statistic {
            Statistic::Quantile(q) => self.per_series(
                &samples,
                RankErrorGT {
                    column: value,
                    fractions: vec![*q],
                },
                ends,
                lookup,
            ),
            Statistic::Entropy => self.per_series(
                &samples,
                KeyedEntropyGT::<f64>::over_columns(value, samples.weight_column()),
                ends,
                lookup,
            ),
            Statistic::Distinct => {
                self.per_series(&samples, CardinalityGT { column: value }, ends, lookup)
            }
            Statistic::Topk { k, by } => {
                let Some(column) = samples.label_names.iter().position(|name| name == by) else {
                    return Ok(Vec::new());
                };
                let mut lists: BTreeMap<i64, Vec<(String, u64)>> = BTreeMap::new();
                for (time, labels, count) in answers {
                    if let Some(key) = labels.get(by) {
                        lists
                            .entry(*time)
                            .or_default()
                            .push((key.clone(), count.round() as u64));
                    }
                }
                let cells = self
                    .windowed(
                        TopkGT::<String>::over_column(*k, column + 1),
                        Vec::new(),
                        ends,
                    )
                    .score(&samples.table, |end, _, probes| {
                        let list = lists.get(&end).cloned().unwrap_or_default();
                        Some(vec![list; probes.len()])
                    })?;
                Ok(cells_of(&samples, cells))
            }
        }
    }

    pub fn reference_rank_errors(
        &self,
        series: &[Series],
        ends: &[i64],
        answers: &[AnswerAt],
        reference: &HashMap<(i64, Labels), f64>,
    ) -> Result<Vec<Cell>> {
        if !matches!(self.statistic, Statistic::Quantile(_)) {
            return Ok(Vec::new());
        }
        let samples = self.samples(series);
        let by_key = keyed(answers);
        self.per_series(
            &samples,
            ReferenceRankErrorGT {
                column: samples.value_column(),
            },
            ends,
            |end, labels| {
                let key = (end, labels.clone());
                Some((*reference.get(&key)?, *by_key.get(&key)?))
            },
        )
    }
}

fn cells_of(samples: &Samples, cells: Vec<WindowScore>) -> Vec<Cell> {
    cells
        .into_iter()
        .map(|cell| (cell.end, samples.labels_of(&cell.group), cell.scores))
        .collect()
}
