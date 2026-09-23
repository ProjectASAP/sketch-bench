use asap_types::pre_asap::expr_ir::{ArithmeticOpKind, CompareOpKind, ScalarValue};
use asap_types::pre_asap::schema::ColumnId;
use asap_types::pre_asap::QueryExpr;
use datafusion::arrow::datatypes::IntervalMonthDayNano;
use datafusion::common::{Column as DfColumn, DFSchemaRef, ScalarValue as DfScalarValue};
use datafusion::functions::expr_fn::{atan2, power, regexp_like};
use datafusion::logical_expr::{binary_expr, Case, Expr, LogicalPlan, Operator};
use datafusion::prelude::lit;

use crate::df::refusal::Refusal;
use crate::df::schema::arrow_type;

#[derive(Debug, Clone)]
pub struct ColumnScope {
    columns: Vec<DfColumn>,
}

impl ColumnScope {
    pub fn of_schema(schema: &DFSchemaRef) -> Self {
        Self {
            columns: schema
                .iter()
                .map(|(qualifier, field)| DfColumn::new(qualifier.cloned(), field.name()))
                .collect(),
        }
    }

    pub fn of_plan(plan: &LogicalPlan) -> Self {
        Self::of_schema(plan.schema())
    }

    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
        }
    }

    pub fn joined(left: &ColumnScope, right: &ColumnScope) -> Self {
        let mut columns = left.columns.clone();
        columns.extend(right.columns.iter().cloned());
        Self { columns }
    }

    pub fn width(&self) -> usize {
        self.columns.len()
    }

    pub fn column(&self, id: ColumnId) -> Result<DfColumn, Refusal> {
        self.columns.get(id).cloned().ok_or_else(|| {
            Refusal::no_constructor(
                "QueryExpr::Column",
                format!(
                    "column position {id} is past the {}-column input of this node",
                    self.columns.len()
                ),
            )
        })
    }

    pub fn expr(&self, id: ColumnId) -> Result<Expr, Refusal> {
        self.column(id).map(Expr::Column)
    }
}

pub fn lower_scalar(expr: &QueryExpr, scope: &ColumnScope) -> Result<Expr, Refusal> {
    match expr {
        QueryExpr::Column(id) => scope.expr(*id),
        QueryExpr::Literal(value) => Ok(lit(literal(value)?)),
        QueryExpr::Compare { left, op, right } => {
            compare(lower_scalar(left, scope)?, op, lower_scalar(right, scope)?)
        }
        QueryExpr::BoolAnd(terms) => fold(terms, scope, Operator::And, true),
        QueryExpr::BoolOr(terms) => fold(terms, scope, Operator::Or, false),
        QueryExpr::Not(inner) => Ok(Expr::Not(Box::new(lower_scalar(inner, scope)?))),
        QueryExpr::IsNull(inner) => Ok(Expr::IsNull(Box::new(lower_scalar(inner, scope)?))),
        QueryExpr::IsNotNull(inner) => Ok(Expr::IsNotNull(Box::new(lower_scalar(inner, scope)?))),
        QueryExpr::Cast {
            expr: inner,
            to,
            try_cast,
        } => {
            let inner = Box::new(lower_scalar(inner, scope)?);
            let to = arrow_type(to)?;
            Ok(if *try_cast {
                Expr::TryCast(datafusion::logical_expr::TryCast {
                    expr: inner,
                    data_type: to,
                })
            } else {
                Expr::Cast(datafusion::logical_expr::Cast {
                    expr: inner,
                    data_type: to,
                })
            })
        }
        QueryExpr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let inner = Box::new(lower_scalar(inner, scope)?);
            let list = list
                .iter()
                .map(|item| lower_scalar(item, scope))
                .collect::<Result<Vec<_>, Refusal>>()?;
            Ok(Expr::InList(datafusion::logical_expr::expr::InList {
                expr: inner,
                list,
                negated: *negated,
            }))
        }
        QueryExpr::FunctionCall { name, args } => {
            let args = args
                .iter()
                .map(|arg| lower_scalar(arg, scope))
                .collect::<Result<Vec<_>, Refusal>>()?;
            Ok(Expr::ScalarFunction(
                datafusion::logical_expr::expr::ScalarFunction::new_udf(
                    named_function(name)?,
                    args,
                ),
            ))
        }
        QueryExpr::Arithmetic { op, left, right } => {
            let left = lower_scalar(left, scope)?;
            let right = lower_scalar(right, scope)?;
            Ok(match op {
                ArithmeticOpKind::Add => binary_expr(left, Operator::Plus, right),
                ArithmeticOpKind::Sub => binary_expr(left, Operator::Minus, right),
                ArithmeticOpKind::Mul => binary_expr(left, Operator::Multiply, right),
                ArithmeticOpKind::Div => binary_expr(left, Operator::Divide, right),
                ArithmeticOpKind::Mod => binary_expr(left, Operator::Modulo, right),
                ArithmeticOpKind::Pow => power(left, right),
                ArithmeticOpKind::Atan2 => atan2(left, right),
            })
        }
        QueryExpr::Case {
            operand,
            branches,
            else_expr,
        } => {
            let operand = operand
                .as_ref()
                .map(|inner| lower_scalar(inner, scope).map(Box::new))
                .transpose()?;
            let when_then_expr = branches
                .iter()
                .map(|(when, then)| {
                    Ok((
                        Box::new(lower_scalar(when, scope)?),
                        Box::new(lower_scalar(then, scope)?),
                    ))
                })
                .collect::<Result<Vec<_>, Refusal>>()?;
            let else_expr = else_expr
                .as_ref()
                .map(|inner| lower_scalar(inner, scope).map(Box::new))
                .transpose()?;
            Ok(Expr::Case(Case {
                expr: operand,
                when_then_expr,
                else_expr,
            }))
        }

        operator => Err(operator_in_scalar_position(operator)),
    }
}

pub fn operator_in_scalar_position(expr: &QueryExpr) -> Refusal {
    Refusal::no_constructor(
        format!("QueryExpr::{}", crate::rows::variant_name(expr)),
        "a scalar position takes a scalar sub-expression; upstream issue #205 dropped the \
         type-level split between the two and no front end constructs a relational operator here",
    )
}

fn fold(
    terms: &[QueryExpr],
    scope: &ColumnScope,
    op: Operator,
    empty: bool,
) -> Result<Expr, Refusal> {
    let mut lowered = terms.iter().map(|term| lower_scalar(term, scope));
    let Some(first) = lowered.next() else {
        return Ok(lit(empty));
    };
    lowered.try_fold(first?, |held, next| Ok(binary_expr(held, op, next?)))
}

fn compare(left: Expr, op: &CompareOpKind, right: Expr) -> Result<Expr, Refusal> {
    Ok(match op {
        CompareOpKind::Eq => binary_expr(left, Operator::Eq, right),
        CompareOpKind::Ne => binary_expr(left, Operator::NotEq, right),
        CompareOpKind::Lt => binary_expr(left, Operator::Lt, right),
        CompareOpKind::Le => binary_expr(left, Operator::LtEq, right),
        CompareOpKind::Gt => binary_expr(left, Operator::Gt, right),
        CompareOpKind::Ge => binary_expr(left, Operator::GtEq, right),
        CompareOpKind::Like => like(left, right, false, false),
        CompareOpKind::NotLike => like(left, right, true, false),
        CompareOpKind::ILike => like(left, right, false, true),
        CompareOpKind::NotILike => like(left, right, true, true),
        CompareOpKind::Regex => regexp_like(left, right, None),
        CompareOpKind::NotRegex => Expr::Not(Box::new(regexp_like(left, right, None))),
    })
}

fn like(left: Expr, right: Expr, negated: bool, case_insensitive: bool) -> Expr {
    Expr::Like(datafusion::logical_expr::expr::Like {
        negated,
        expr: Box::new(left),
        pattern: Box::new(right),
        escape_char: None,
        case_insensitive,
    })
}

pub fn literal(value: &ScalarValue) -> Result<DfScalarValue, Refusal> {
    Ok(match value {
        ScalarValue::Int64(held) => DfScalarValue::Int64(Some(*held)),
        ScalarValue::Float64(held) => DfScalarValue::Float64(Some(*held)),
        ScalarValue::Utf8(held) => DfScalarValue::Utf8(Some(held.clone())),
        ScalarValue::Boolean(held) => DfScalarValue::Boolean(Some(*held)),
        ScalarValue::Null => DfScalarValue::Null,
        ScalarValue::Interval {
            months,
            days,
            nanos,
        } => DfScalarValue::IntervalMonthDayNano(Some(IntervalMonthDayNano::new(
            *months, *days, *nanos,
        ))),
    })
}

fn named_function(
    name: &str,
) -> Result<std::sync::Arc<datafusion::logical_expr::ScalarUDF>, Refusal> {
    registry().get(name).cloned().ok_or_else(|| {
        Refusal::deferred(
            format!("QueryExpr::FunctionCall({name})"),
            "scalar-function-registry",
            format!(
                "{name} is not one of the scalar functions a default DataFusion session registers"
            ),
        )
    })
}

fn registry(
) -> &'static std::collections::HashMap<String, std::sync::Arc<datafusion::logical_expr::ScalarUDF>>
{
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<
        std::collections::HashMap<String, std::sync::Arc<datafusion::logical_expr::ScalarUDF>>,
    > = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut named = std::collections::HashMap::new();
        for function in datafusion::functions::all_default_functions() {
            for alias in function.aliases() {
                named.insert(alias.clone(), function.clone());
            }
            named.insert(function.name().to_string(), function);
        }
        for function in datafusion::functions_nested::all_default_nested_functions() {
            for alias in function.aliases() {
                named.insert(alias.clone(), function.clone());
            }
            named.insert(function.name().to_string(), function);
        }
        named
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::df::variants::{every_query_expr, leaf_scope, LATENCY};
    use crate::rows::variant_name;
    use std::rc::Rc;

    const ADMITTED: [&str; 13] = [
        "Arithmetic",
        "BoolAnd",
        "BoolOr",
        "Case",
        "Cast",
        "Column",
        "Compare",
        "FunctionCall",
        "InList",
        "IsNotNull",
        "IsNull",
        "Literal",
        "Not",
    ];

    const DEFERRED: [&str; 23] = [
        "Aggregate",
        "BinaryOp",
        "Concat",
        "CurrentTimestamp",
        "Dedup",
        "EvalTimestamp",
        "Filter",
        "Join",
        "Limit",
        "Project",
        "PromqlInfoEnrich",
        "PromqlRelabel",
        "PromqlScalarBridge",
        "PromqlScalarFromVector",
        "PromqlSeriesSample",
        "PromqlSubquery",
        "PromqlVectorFromScalar",
        "SQLWindowFunc",
        "Scan",
        "SetOp",
        "Sort",
        "TimeRange",
        "TimeShift",
    ];

    fn sorted(names: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
        let mut held: Vec<&'static str> = names.into_iter().collect();
        held.sort_unstable();
        held
    }

    #[test]
    fn the_two_tables_name_every_variant_of_the_tree_exactly_once() {
        let mut both = sorted(ADMITTED.into_iter().chain(DEFERRED));
        let before = both.len();
        both.dedup();
        assert_eq!(both.len(), before, "a name appears in both tables");
        assert_eq!(before, 36, "the tree has 36 variants");
        assert_eq!(
            both,
            sorted(every_query_expr().iter().map(|(name, _)| *name))
        );
    }

    #[test]
    fn every_name_in_the_two_tables_is_the_one_variant_name_gives_that_tree() {
        for (name, tree) in every_query_expr() {
            assert_eq!(variant_name(&tree), name);
        }
    }

    #[test]
    fn every_admitted_variant_lowers_to_an_expression() {
        let scope = leaf_scope();
        for (name, tree) in every_query_expr() {
            if !ADMITTED.contains(&name) {
                continue;
            }
            lower_scalar(&tree, &scope)
                .unwrap_or_else(|refusal| panic!("{name} is admitted: {refusal}"));
        }
    }

    #[test]
    fn every_deferred_variant_is_refused_by_name_with_a_producer_side_reason() {
        let scope = leaf_scope();
        for (name, tree) in every_query_expr() {
            if !DEFERRED.contains(&name) {
                continue;
            }
            let refusal =
                lower_scalar(&tree, &scope).expect_err("a relational node has no scalar meaning");
            assert_eq!(refusal.variant, format!("QueryExpr::{name}"));
            assert_eq!(refusal.tag(), "no_constructor", "{name}");
        }
    }

    #[test]
    fn an_empty_conjunction_is_true_and_an_empty_disjunction_is_false() {
        let scope = leaf_scope();
        assert_eq!(
            lower_scalar(&QueryExpr::BoolAnd(Vec::new()), &scope).unwrap(),
            lit(true)
        );
        assert_eq!(
            lower_scalar(&QueryExpr::BoolOr(Vec::new()), &scope).unwrap(),
            lit(false)
        );
    }

    #[test]
    fn a_column_past_the_input_width_is_refused_by_position() {
        let scope = leaf_scope();
        let refusal = lower_scalar(&QueryExpr::Column(scope.width()), &scope).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::Column");
        assert_eq!(refusal.tag(), "no_constructor");
    }

    #[test]
    fn a_function_name_outside_the_registry_is_deferred_by_name() {
        let scope = leaf_scope();
        let tree = QueryExpr::FunctionCall {
            name: "no_such_function".to_owned(),
            args: vec![QueryExpr::Column(LATENCY)],
        };
        let refusal = lower_scalar(&tree, &scope).expect_err("refuses");
        assert_eq!(refusal.variant, "QueryExpr::FunctionCall(no_such_function)");
        assert_eq!(refusal.tag(), "deferred");
    }

    #[test]
    fn every_comparison_operator_has_an_expression() {
        let scope = leaf_scope();
        for op in [
            CompareOpKind::Eq,
            CompareOpKind::Ne,
            CompareOpKind::Lt,
            CompareOpKind::Le,
            CompareOpKind::Gt,
            CompareOpKind::Ge,
            CompareOpKind::Like,
            CompareOpKind::NotLike,
            CompareOpKind::ILike,
            CompareOpKind::NotILike,
            CompareOpKind::Regex,
            CompareOpKind::NotRegex,
        ] {
            let tree = QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(LATENCY)),
                op: op.clone(),
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(1.0))),
            };
            lower_scalar(&tree, &scope).unwrap_or_else(|refusal| panic!("{op}: {refusal}"));
        }
    }

    #[test]
    fn every_arithmetic_operator_has_an_expression() {
        let scope = leaf_scope();
        for op in [
            ArithmeticOpKind::Add,
            ArithmeticOpKind::Sub,
            ArithmeticOpKind::Mul,
            ArithmeticOpKind::Div,
            ArithmeticOpKind::Mod,
            ArithmeticOpKind::Pow,
            ArithmeticOpKind::Atan2,
        ] {
            let tree = QueryExpr::Arithmetic {
                op: op.clone(),
                left: Rc::new(QueryExpr::Column(LATENCY)),
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(2.0))),
            };
            lower_scalar(&tree, &scope).unwrap_or_else(|refusal| panic!("{op}: {refusal}"));
        }
    }

    #[test]
    fn every_literal_shape_has_a_datafusion_scalar() {
        assert_eq!(
            literal(&ScalarValue::Int64(7)).unwrap(),
            DfScalarValue::Int64(Some(7))
        );
        assert_eq!(
            literal(&ScalarValue::Float64(1.5)).unwrap(),
            DfScalarValue::Float64(Some(1.5))
        );
        assert_eq!(
            literal(&ScalarValue::Utf8("a".to_owned())).unwrap(),
            DfScalarValue::Utf8(Some("a".to_owned()))
        );
        assert_eq!(
            literal(&ScalarValue::Boolean(true)).unwrap(),
            DfScalarValue::Boolean(Some(true))
        );
        assert_eq!(literal(&ScalarValue::Null).unwrap(), DfScalarValue::Null);
        assert_eq!(
            literal(&ScalarValue::Interval {
                months: 1,
                days: 2,
                nanos: 3
            })
            .unwrap(),
            DfScalarValue::IntervalMonthDayNano(Some(IntervalMonthDayNano::new(1, 2, 3)))
        );
    }
}
