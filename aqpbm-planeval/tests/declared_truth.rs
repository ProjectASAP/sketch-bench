use std::collections::BTreeMap;
use std::path::Path;

use aqpbm_planeval::plan::{workload, Arm};
use aqpbm_planeval::spec::{Spec, Statistic, Truth};
use asap_types::ir::operator::{AggIntent, Reduction, Source};
use asap_types::ir::scalar::{CompareOpKind, ScalarValue};
use asap_types::ir::{NonASAPOp, Operator, OperatorNode, QueryRoot, ScalarExpr, TimeRangeKind};
use asap_types::workload::PlanningWorkload;

fn aggregate(node: &OperatorNode) -> Option<(&Reduction, &AggIntent, &OperatorNode)> {
    let Operator::NonASAP(NonASAPOp::Aggregate {
        reduction,
        measures,
        filters,
        having,
        child,
        ..
    }) = &node.operator
    else {
        return None;
    };
    let [measure] = measures.as_slice() else {
        return None;
    };
    if having.is_some() || filters.iter().any(Option::is_some) {
        return None;
    }
    Some((reduction, measure, child))
}

fn grouped_by(reduction: &Reduction) -> Option<&[usize]> {
    match reduction {
        Reduction::Reduce(keys) if !keys.is_without() => Some(keys.keys()),
        _ => None,
    }
}

fn lowered(workload: &PlanningWorkload) -> Option<(Truth, i64)> {
    let roots = asap_frontend_promql::lower_promql_query_workload(workload).unwrap();
    let Some(QueryRoot::Operator(root)) = roots.first() else {
        return None;
    };
    let (reduction, measure, child) = aggregate(root)?;
    match (reduction, measure) {
        (Reduction::PerEntity, AggIntent::Quantile { col: None, q, .. }) => {
            over(Statistic::Quantile(*q), child, None)
        }
        (Reduction::PerEntity, AggIntent::FrequencyEntropy { col: None, .. }) => {
            over(Statistic::Entropy, child, None)
        }
        (Reduction::PerEntity, AggIntent::Cardinality { cols, .. }) if cols.is_empty() => {
            over(Statistic::Distinct, child, None)
        }
        (reduction, AggIntent::TopK { k, .. }) if grouped_by(reduction) == Some(&[]) => {
            top_k(*k, child)
        }
        _ => None,
    }
}

fn top_k(k: usize, node: &OperatorNode) -> Option<(Truth, i64)> {
    let (reduction, AggIntent::Sum { col: None, .. }, child) = aggregate(node)? else {
        return None;
    };
    let &[by] = grouped_by(reduction)? else {
        return None;
    };
    let (Reduction::PerEntity, AggIntent::Count { .. }, child) = aggregate(child)? else {
        return None;
    };
    over(
        Statistic::TopK {
            k,
            by: String::new(),
        },
        child,
        Some(by),
    )
}

fn over(statistic: Statistic, node: &OperatorNode, by: Option<usize>) -> Option<(Truth, i64)> {
    let Operator::NonASAP(NonASAPOp::TimeRange {
        range,
        kind: TimeRangeKind::Range,
        child,
    }) = &node.operator
    else {
        return None;
    };
    let Operator::NonASAP(NonASAPOp::Scan {
        source: Source::TimeSeries { metric },
        predicates,
        schema,
    }) = &child.operator
    else {
        return None;
    };
    let name = |column: usize| schema.fields.get(column).map(|f| f.name.clone());
    let matchers = predicates
        .iter()
        .map(|predicate| match &predicate.0 {
            ScalarExpr::Compare {
                left,
                op: CompareOpKind::Eq,
                right,
                ..
            } => match (left.as_ref(), right.as_ref()) {
                (ScalarExpr::Column(column), ScalarExpr::Literal(ScalarValue::Utf8(value))) => {
                    Some((name(*column)?, value.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect::<Option<BTreeMap<_, _>>>()?;
    let statistic = match (statistic, by) {
        (Statistic::TopK { k, .. }, Some(column)) => Statistic::TopK {
            k,
            by: name(column)?,
        },
        (statistic, _) => statistic,
    };
    Some((
        Truth {
            statistic,
            metric: metric.clone(),
            matchers,
        },
        range.as_millis() as i64,
    ))
}

#[test]
fn every_workload_declares_the_truth_its_lowered_query_has() {
    let exact = Arm {
        max_error: None,
        failure_probability: 0.01,
        raw_data_retained: true,
        ignore_guarantees: false,
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(Path::new("workloads")).unwrap() {
        let path = entry.unwrap().path();
        let spec = Spec::read(&path).unwrap();
        for query in &spec.queries {
            let declared = query.truth.clone().map(|truth| (truth, query.window_ms));
            assert_eq!(
                declared,
                lowered(&workload(&spec, query, &exact).unwrap()),
                "{} {}",
                path.display(),
                query.id
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
}
