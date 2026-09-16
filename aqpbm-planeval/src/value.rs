use std::rc::Rc;

use asap_types::post_asap::{ExactOperation, PostAsapNodeId, ValueOperation};
use asap_types::pre_asap::{ProjectItem, QueryExpr, SortKey};

use crate::rows::{eval, order, variant_name};
use crate::types::{EvalError, Refusal, Row, Value};

/// Everything about a row operation that can be settled before a row is read.
///
/// Matched arm for arm against [`apply`], so a variant that grew one and a
/// variant that grew the other cannot drift apart silently.
pub(crate) fn check(
    node: PostAsapNodeId,
    operation: &ValueOperation,
    columns: usize,
) -> Result<(), Refusal> {
    match operation {
        ValueOperation::Filter { pred } => crate::rows::check_predicate(&pred.0, columns)
            .map_err(|fault| refusal(node, format!("Filter: {}", fault.detail()))),
        ValueOperation::Project { cols, .. } => project_shape(node, cols),
        ValueOperation::Sort { keys, partition_by } => {
            if keys.is_empty() {
                return Err(refusal(node, "Sort carries no key".to_owned()));
            }
            if !partition_by.is_empty() {
                return Err(refusal(
                    node,
                    format!(
                        "Sort partitioned by {partition_by:?}: the rows carry no partition to \
                         sort within"
                    ),
                ));
            }
            for key in keys {
                crate::rows::check_predicate(&key.expr, columns)
                    .map_err(|fault| refusal(node, format!("Sort key: {}", fault.detail())))?;
            }
            Ok(())
        }
        ValueOperation::Limit { .. } => Ok(()),
        ValueOperation::FinalizeExactAccumulator => Err(refusal(
            node,
            "FinalizeExactAccumulator reads summary state, and a row operation is handed rows"
                .to_owned(),
        )),
        ValueOperation::Exact(exact) => match exact {
            ExactOperation::Aggregate { .. } => Err(refusal(
                node,
                "Exact(Aggregate) is a whole aggregation engine, not a row operation".to_owned(),
            )),
            other => Err(refusal(node, format!("Exact({})", debug_variant(other)))),
        },
        ValueOperation::MaintainPopulation { .. } | ValueOperation::ReadPopulation { .. } => {
            Err(refusal(
                node,
                format!(
                    "{}: a maintained population is state this crate holds none of",
                    debug_variant(operation)
                ),
            ))
        }
        ValueOperation::Extension { name } => Err(Refusal::UnregisteredExtension {
            node,
            name: name.clone(),
        }),
        other => Err(refusal(node, debug_variant(other))),
    }
}

pub(crate) fn apply(
    node: PostAsapNodeId,
    operation: &ValueOperation,
    rows: &Rc<Vec<Row>>,
) -> Result<Rc<Vec<Row>>, EvalError> {
    match operation {
        ValueOperation::Filter { pred } => {
            let mut kept = Vec::new();
            for row in rows.iter() {
                if crate::rows::passes(&pred.0, row)? {
                    kept.push(row.clone());
                }
            }
            Ok(Rc::new(kept))
        }
        ValueOperation::Project { cols, .. } => project(node, cols, rows),
        ValueOperation::Sort { keys, partition_by } => {
            if !partition_by.is_empty() {
                return Err(refused(
                    node,
                    format!(
                        "Sort partitioned by {:?}: the rows carry no partition to sort within",
                        partition_by
                    ),
                ));
            }
            sort(node, keys, rows)
        }
        ValueOperation::Limit { n, offset } => {
            let start = (*offset).min(rows.len());
            let end = offset.saturating_add(*n).min(rows.len());
            Ok(Rc::new(rows[start..end].to_vec()))
        }
        ValueOperation::FinalizeExactAccumulator => Err(refused(
            node,
            "FinalizeExactAccumulator reads summary state, and this node was handed rows"
                .to_owned(),
        )),
        ValueOperation::Exact(exact) => match exact {
            ExactOperation::Aggregate { .. } => Err(refused(
                node,
                "Exact(Aggregate) is a whole aggregation engine, not a row operation".to_owned(),
            )),
            other => Err(refused(node, format!("Exact({})", debug_variant(other)))),
        },
        ValueOperation::MaintainPopulation { .. } | ValueOperation::ReadPopulation { .. } => {
            Err(refused(
                node,
                format!(
                    "{}: a maintained population is state this crate holds none of",
                    debug_variant(operation)
                ),
            ))
        }
        ValueOperation::Extension { name } => {
            Err(EvalError::Refused(vec![Refusal::UnregisteredExtension {
                node,
                name: name.clone(),
            }]))
        }
        other => Err(refused(node, debug_variant(other))),
    }
}

fn project_shape(node: PostAsapNodeId, cols: &[ProjectItem]) -> Result<(), Refusal> {
    for (index, item) in cols.iter().enumerate() {
        match &item.expr {
            QueryExpr::Column(_) | QueryExpr::Literal(_) => {}
            other => {
                return Err(refusal(
                    node,
                    format!("Project col {index} is a {}", variant_name(other)),
                ))
            }
        }
    }
    Ok(())
}

fn project(
    node: PostAsapNodeId,
    cols: &[ProjectItem],
    rows: &Rc<Vec<Row>>,
) -> Result<Rc<Vec<Row>>, EvalError> {
    project_shape(node, cols).map_err(|refusal| EvalError::Refused(vec![refusal]))?;
    let mut projected = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        let mut out = Vec::with_capacity(cols.len());
        for item in cols {
            out.push(eval(&item.expr, row)?);
        }
        projected.push(Row(out));
    }
    Ok(Rc::new(projected))
}

fn sort(
    node: PostAsapNodeId,
    keys: &[SortKey],
    rows: &Rc<Vec<Row>>,
) -> Result<Rc<Vec<Row>>, EvalError> {
    if keys.is_empty() {
        return Err(refused(node, "Sort carries no key".to_owned()));
    }
    let mut keyed: Vec<(Vec<Value>, &Row)> = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        let mut computed = Vec::with_capacity(keys.len());
        for key in keys {
            computed.push(eval(&key.expr, row)?);
        }
        keyed.push((computed, row));
    }

    keyed.sort_by(|left, right| {
        for (index, key) in keys.iter().enumerate() {
            let ordering = ranked(
                &left.0[index],
                &right.0[index],
                key.ascending,
                key.nulls_first,
            );
            if ordering.is_ne() {
                return ordering;
            }
        }
        std::cmp::Ordering::Equal
    });

    Ok(Rc::new(
        keyed.into_iter().map(|(_, row)| row.clone()).collect(),
    ))
}

fn ranked(left: &Value, right: &Value, ascending: bool, nulls_first: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (matches!(left, Value::Null), matches!(right, Value::Null)) {
        (true, true) => Ordering::Equal,
        (true, false) if nulls_first => Ordering::Less,
        (true, false) => Ordering::Greater,
        (false, true) if nulls_first => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let ordering = order(left, right);
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        }
    }
}

fn refusal(node: PostAsapNodeId, detail: String) -> Refusal {
    Refusal::UnsupportedValueOperation { node, detail }
}

fn refused(node: PostAsapNodeId, detail: String) -> EvalError {
    EvalError::Refused(vec![refusal(node, detail)])
}

fn debug_variant<T: std::fmt::Debug>(value: &T) -> String {
    let rendered = format!("{value:?}");
    rendered
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|piece| !piece.is_empty())
        .unwrap_or("<unnameable>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_types::pre_asap::{CompareOpKind, GroupKeys, Predicate, ScalarValue};

    fn rows(values: &[f64]) -> Rc<Vec<Row>> {
        Rc::new(
            values
                .iter()
                .enumerate()
                .map(|(i, value)| Row(vec![Value::Timestamp(i as i64), Value::Float(*value)]))
                .collect(),
        )
    }

    fn values(rows: &Rc<Vec<Row>>) -> Vec<f64> {
        rows.iter()
            .map(|row| match row.0[1] {
                Value::Float(value) => value,
                ref other => panic!("{other:?}"),
            })
            .collect()
    }

    fn node() -> PostAsapNodeId {
        PostAsapNodeId(7)
    }

    fn sort_by_value(ascending: bool) -> ValueOperation {
        ValueOperation::Sort {
            keys: vec![SortKey {
                expr: QueryExpr::Column(1),
                ascending,
                nulls_first: false,
            }],
            partition_by: GroupKeys::default(),
        }
    }

    #[test]
    fn sort_then_limit_is_the_shape_promql_topk_compiles_to() {
        let input = rows(&[3.0, 9.0, 1.0, 7.0, 5.0]);

        let sorted = apply(node(), &sort_by_value(false), &input).expect("sorts");
        assert_eq!(values(&sorted), vec![9.0, 7.0, 5.0, 3.0, 1.0]);

        let limited =
            apply(node(), &ValueOperation::Limit { n: 2, offset: 0 }, &sorted).expect("limits");
        assert_eq!(values(&limited), vec![9.0, 7.0]);

        let ascending = apply(node(), &sort_by_value(true), &input).expect("sorts");
        assert_eq!(values(&ascending), vec![1.0, 3.0, 5.0, 7.0, 9.0]);
    }

    #[test]
    fn a_limit_past_the_end_is_the_rows_that_exist_not_an_error() {
        let input = rows(&[1.0, 2.0]);
        for (n, offset, expected) in [
            (10usize, 0usize, vec![1.0, 2.0]),
            (1, 1, vec![2.0]),
            (5, 9, vec![]),
            (0, 0, vec![]),
        ] {
            let held = apply(node(), &ValueOperation::Limit { n, offset }, &input).expect("limits");
            assert_eq!(values(&held), expected, "n={n} offset={offset}");
        }
    }

    #[test]
    fn a_filter_keeps_the_rows_its_predicate_passes() {
        let input = rows(&[1.0, 5.0, 9.0]);
        let operation = ValueOperation::Filter {
            pred: Predicate(Rc::new(QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(1)),
                op: CompareOpKind::Gt,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(4.0))),
            })),
        };
        let kept = apply(node(), &operation, &input).expect("filters");
        assert_eq!(values(&kept), vec![5.0, 9.0]);
    }

    #[test]
    fn a_projection_narrows_the_row_to_the_named_columns() {
        let input = rows(&[1.0, 2.0]);
        let operation = ValueOperation::Project {
            cols: vec![ProjectItem {
                alias: None,
                expr: QueryExpr::Column(1),
            }],
            qualifier: None,
        };
        let projected = apply(node(), &operation, &input).expect("projects");
        assert!(projected.iter().all(|row| row.0.len() == 1));
        assert_eq!(projected[1].0[0], Value::Float(2.0));
    }

    #[test]
    fn debug_variant_names_a_variant_from_any_shape() {
        assert_eq!(
            debug_variant(&ValueOperation::Limit { n: 1, offset: 0 }),
            "Limit"
        );
        assert_eq!(
            debug_variant(&ValueOperation::FinalizeExactAccumulator),
            "FinalizeExactAccumulator"
        );
        assert_eq!(
            debug_variant(&ValueOperation::Extension { name: "x".into() }),
            "Extension"
        );
    }

    #[test]
    fn the_operations_with_no_row_semantics_are_refused_by_name() {
        let input = rows(&[1.0]);
        for operation in [
            ValueOperation::FinalizeExactAccumulator,
            ValueOperation::Sort {
                keys: vec![SortKey {
                    expr: QueryExpr::Column(1),
                    ascending: true,
                    nulls_first: false,
                }],
                partition_by: GroupKeys::by(vec![0]),
            },
            ValueOperation::Extension {
                name: "promql_histogram_quantile".into(),
            },
        ] {
            let err = apply(node(), &operation, &input).expect_err("is refused");
            assert!(
                matches!(err, EvalError::Refused(_)),
                "{operation:?}: {err:?}"
            );
        }
    }
}
