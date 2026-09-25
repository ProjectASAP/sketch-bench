use asap_types::pre_asap::agg_intent::AggIntent;
use asap_types::pre_asap::schema::ColumnId;
use datafusion::arrow::datatypes::DataType as ArrowDataType;
use datafusion::common::ScalarValue as DfScalarValue;
use datafusion::functions_aggregate::expr_fn::{
    array_agg, avg, corr, count, count_distinct, max, min, stddev, stddev_pop, sum, var_pop,
    var_sample,
};
use datafusion::functions_nested::expr_fn::{array_element, array_length, array_sort};
use datafusion::logical_expr::{col, lit, when, Expr, ExprFunctionExt};

use crate::df::refusal::Refusal;
use crate::df::scalar::ColumnScope;

#[derive(Debug, Clone)]
pub struct LoweredMeasure {
    pub aggregated: Expr,
    pub readout: Option<Expr>,
}

impl LoweredMeasure {
    fn direct(aggregated: Expr) -> Self {
        Self {
            aggregated,
            readout: None,
        }
    }
}

pub fn intent_name(intent: &AggIntent) -> &'static str {
    match intent {
        AggIntent::Count { .. } => "AggIntent::Count",
        AggIntent::Sum { .. } => "AggIntent::Sum",
        AggIntent::Min { .. } => "AggIntent::Min",
        AggIntent::Max { .. } => "AggIntent::Max",
        AggIntent::Avg { .. } => "AggIntent::Avg",
        AggIntent::StdDev { .. } => "AggIntent::StdDev",
        AggIntent::Variance { .. } => "AggIntent::Variance",
        AggIntent::PearsonCorr { .. } => "AggIntent::PearsonCorr",
        AggIntent::Quantile { .. } => "AggIntent::Quantile",
        AggIntent::TopK { .. } => "AggIntent::TopK",
        AggIntent::Cardinality { .. } => "AggIntent::Cardinality",
        AggIntent::FrequencyL2 { .. } => "AggIntent::FrequencyL2",
        AggIntent::FrequencyEntropy { .. } => "AggIntent::FrequencyEntropy",
        AggIntent::Rate => "AggIntent::Rate",
        AggIntent::IRate => "AggIntent::IRate",
        AggIntent::Increase => "AggIntent::Increase",
        AggIntent::Changes => "AggIntent::Changes",
        AggIntent::Delta => "AggIntent::Delta",
        AggIntent::IDelta => "AggIntent::IDelta",
        AggIntent::Deriv => "AggIntent::Deriv",
        AggIntent::Resets => "AggIntent::Resets",
        AggIntent::PredictLinear { .. } => "AggIntent::PredictLinear",
        AggIntent::DoubleExpSmoothing { .. } => "AggIntent::DoubleExpSmoothing",
        AggIntent::HistogramCount => "AggIntent::HistogramCount",
        AggIntent::HistogramSum => "AggIntent::HistogramSum",
        AggIntent::HistogramAvg => "AggIntent::HistogramAvg",
        AggIntent::HistogramStdDev => "AggIntent::HistogramStdDev",
        AggIntent::HistogramStdVar => "AggIntent::HistogramStdVar",
        AggIntent::HistogramFraction { .. } => "AggIntent::HistogramFraction",
        AggIntent::HistogramQuantile { .. } => "AggIntent::HistogramQuantile",
        AggIntent::Math(_) => "AggIntent::Math",
        AggIntent::Absent => "AggIntent::Absent",
        AggIntent::AbsentOverTime => "AggIntent::AbsentOverTime",
        AggIntent::PresentOverTime => "AggIntent::PresentOverTime",
        AggIntent::TimeFn(_) => "AggIntent::TimeFn",
        AggIntent::Group => "AggIntent::Group",
        AggIntent::CountValues { .. } => "AggIntent::CountValues",
        AggIntent::LastOverTime => "AggIntent::LastOverTime",
        AggIntent::FirstOverTime => "AggIntent::FirstOverTime",
        AggIntent::MadOverTime => "AggIntent::MadOverTime",
        AggIntent::TsOfMinOverTime => "AggIntent::TsOfMinOverTime",
        AggIntent::TsOfMaxOverTime => "AggIntent::TsOfMaxOverTime",
        AggIntent::TsOfFirstOverTime => "AggIntent::TsOfFirstOverTime",
        AggIntent::TsOfLastOverTime => "AggIntent::TsOfLastOverTime",
        AggIntent::Extension { .. } => "AggIntent::Extension",
    }
}

pub fn lower_measure(
    intent: &AggIntent,
    scope: &ColumnScope,
    state: &str,
) -> Result<LoweredMeasure, Refusal> {
    match intent {
        AggIntent::Count { .. } => Ok(LoweredMeasure::direct(count(lit(1i64)))),
        AggIntent::Sum { col } => Ok(LoweredMeasure::direct(sum(reduced(
            col,
            scope,
            intent_name(intent),
        )?))),
        AggIntent::Min { col } => Ok(LoweredMeasure::direct(min(reduced(
            col,
            scope,
            intent_name(intent),
        )?))),
        AggIntent::Max { col } => Ok(LoweredMeasure::direct(max(reduced(
            col,
            scope,
            intent_name(intent),
        )?))),
        AggIntent::Avg { col } => Ok(LoweredMeasure::direct(avg(reduced(
            col,
            scope,
            intent_name(intent),
        )?))),
        AggIntent::StdDev { col, population } => {
            let column = reduced(col, scope, intent_name(intent))?;
            Ok(LoweredMeasure::direct(if *population {
                stddev_pop(column)
            } else {
                stddev(column)
            }))
        }
        AggIntent::Variance { col, population } => {
            let column = reduced(col, scope, intent_name(intent))?;
            Ok(LoweredMeasure::direct(if *population {
                var_pop(column)
            } else {
                var_sample(column)
            }))
        }
        AggIntent::PearsonCorr { left, right } => Ok(LoweredMeasure::direct(corr(
            scope.expr(*left)?,
            scope.expr(*right)?,
        ))),
        AggIntent::Quantile {
            col: retained, q, ..
        } => {
            let column = reduced(retained, scope, intent_name(intent))?;
            if !(0.0..=1.0).contains(q) {
                return Err(Refusal::no_constructor(
                    intent_name(intent),
                    format!("q = {q} is outside [0, 1] and names no position in a sorted column"),
                ));
            }
            let aggregated = array_agg(column.clone())
                .filter(column.is_not_null())
                .build()
                .map_err(|error| {
                    Refusal::deferred(
                        intent_name(intent),
                        "array-agg-builder",
                        format!("the retained column could not be built: {error}"),
                    )
                })?;
            Ok(LoweredMeasure {
                aggregated,
                readout: Some(sorted_column_at_rank(col(state), *q)?),
            })
        }
        AggIntent::Cardinality { col, .. } => Ok(LoweredMeasure::direct(count_distinct(reduced(
            col,
            scope,
            intent_name(intent),
        )?))),

        AggIntent::FrequencyL2 { .. } => Err(Refusal::deferred(
            intent_name(intent),
            "aggregate-over-aggregate",
            "the second moment of a value-frequency distribution is a grouped count feeding a \
             second reduction, which is a plan rewrite rather than one aggregate expression",
        )),
        AggIntent::FrequencyEntropy { .. } => Err(Refusal::deferred(
            intent_name(intent),
            "aggregate-over-aggregate",
            "the entropy of a value-frequency distribution is a grouped count feeding a second \
             reduction, which is a plan rewrite rather than one aggregate expression",
        )),
        AggIntent::TopK { .. } => Err(Refusal::promql_only(
            intent_name(intent),
            "SQL spells top-k as Sort + Limit and its front end never emits this intent",
        )),

        AggIntent::Rate => Err(order_sensitive(intent_name(intent))),
        AggIntent::IRate => Err(order_sensitive(intent_name(intent))),
        AggIntent::Increase => Err(order_sensitive(intent_name(intent))),

        AggIntent::Changes => Err(time_axis(intent_name(intent))),
        AggIntent::Delta => Err(time_axis(intent_name(intent))),
        AggIntent::IDelta => Err(time_axis(intent_name(intent))),
        AggIntent::Deriv => Err(time_axis(intent_name(intent))),
        AggIntent::Resets => Err(time_axis(intent_name(intent))),
        AggIntent::PredictLinear { .. } => Err(time_axis(intent_name(intent))),
        AggIntent::DoubleExpSmoothing { .. } => Err(time_axis(intent_name(intent))),

        AggIntent::HistogramCount => Err(histogram(intent_name(intent))),
        AggIntent::HistogramSum => Err(histogram(intent_name(intent))),
        AggIntent::HistogramAvg => Err(histogram(intent_name(intent))),
        AggIntent::HistogramStdDev => Err(histogram(intent_name(intent))),
        AggIntent::HistogramStdVar => Err(histogram(intent_name(intent))),
        AggIntent::HistogramFraction { .. } => Err(histogram(intent_name(intent))),
        AggIntent::HistogramQuantile { .. } => Err(histogram(intent_name(intent))),

        AggIntent::Math(_) => Err(promql_measure(intent_name(intent))),
        AggIntent::Absent => Err(promql_measure(intent_name(intent))),
        AggIntent::AbsentOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::PresentOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::TimeFn(_) => Err(promql_measure(intent_name(intent))),
        AggIntent::Group => Err(promql_measure(intent_name(intent))),
        AggIntent::CountValues { .. } => Err(promql_measure(intent_name(intent))),
        AggIntent::LastOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::FirstOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::MadOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::TsOfMinOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::TsOfMaxOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::TsOfFirstOverTime => Err(promql_measure(intent_name(intent))),
        AggIntent::TsOfLastOverTime => Err(promql_measure(intent_name(intent))),

        AggIntent::Extension { ext_kind, .. } => Err(Refusal::deferred(
            intent_name(intent),
            "dialect-extension",
            format!(
                "{ext_kind} is a dialect measure this step has no built-in aggregate for (G10)"
            ),
        )),
    }
}

pub fn sorted_column_at_rank(retained: Expr, q: f64) -> Result<Expr, Refusal> {
    let length = array_length(retained.clone());
    let length_index = cast_int64(length.clone());
    let position = cast_int64(
        datafusion::functions::expr_fn::floor(lit(q) * cast_float64(length)) + lit(1.0_f64),
    );
    let rank = when(
        position.clone().gt(length_index.clone()),
        length_index.clone(),
    )
    .otherwise(position)
    .map_err(|error| {
        Refusal::deferred(
            "AggIntent::Quantile",
            "rank-expression",
            format!("the rank position could not be built: {error}"),
        )
    })?;
    when(
        length_index.gt(lit(0i64)),
        array_element(array_sort_ascending(retained), rank),
    )
    .otherwise(lit(DfScalarValue::Null))
    .map_err(|error| {
        Refusal::deferred(
            "AggIntent::Quantile",
            "empty-group-readout",
            format!("the empty-group branch could not be built: {error}"),
        )
    })
}

fn array_sort_ascending(retained: Expr) -> Expr {
    array_sort(retained, lit("ASC"), lit("NULLS LAST"))
}

fn cast_int64(expr: Expr) -> Expr {
    Expr::Cast(datafusion::logical_expr::Cast {
        expr: Box::new(expr),
        data_type: ArrowDataType::Int64,
    })
}

fn cast_float64(expr: Expr) -> Expr {
    Expr::Cast(datafusion::logical_expr::Cast {
        expr: Box::new(expr),
        data_type: ArrowDataType::Float64,
    })
}

fn reduced(column: &Option<ColumnId>, scope: &ColumnScope, measure: &str) -> Result<Expr, Refusal> {
    match column {
        Some(id) => scope.expr(*id),
        None => Err(Refusal::no_constructor(
            measure,
            "the measure carries no column, and the SQL front end always names one",
        )),
    }
}

fn order_sensitive(variant: &str) -> Refusal {
    Refusal::deferred(
        variant,
        "order-sensitive",
        "a counter-reset-aware measure reads its input in timestamp order, which needs an \
         aggregate declaring a beneficial ordering",
    )
}

fn time_axis(variant: &str) -> Refusal {
    Refusal::time_axis(
        variant,
        "the measure reads its input along a time axis the DAG carries no evaluation instant for",
    )
}

fn histogram(variant: &str) -> Refusal {
    Refusal::promql_only(
        variant,
        "a native histogram is a PromQL sample type the SQL front end never produces",
    )
}

fn promql_measure(variant: &str) -> Refusal {
    Refusal::promql_only(
        variant,
        "the measure exists to serve a PromQL function the SQL front end never lowers to",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::df::variants::{every_agg_intent, leaf_scope, ACCURACY, LATENCY};

    const STATE: &str = "__asap_m0";

    const ADMITTED: [&str; 10] = [
        "AggIntent::Avg",
        "AggIntent::Cardinality",
        "AggIntent::Count",
        "AggIntent::Max",
        "AggIntent::Min",
        "AggIntent::PearsonCorr",
        "AggIntent::Quantile",
        "AggIntent::StdDev",
        "AggIntent::Sum",
        "AggIntent::Variance",
    ];

    const DEFERRED: [&str; 35] = [
        "AggIntent::Absent",
        "AggIntent::AbsentOverTime",
        "AggIntent::Changes",
        "AggIntent::CountValues",
        "AggIntent::Delta",
        "AggIntent::Deriv",
        "AggIntent::DoubleExpSmoothing",
        "AggIntent::Extension",
        "AggIntent::FirstOverTime",
        "AggIntent::FrequencyEntropy",
        "AggIntent::FrequencyL2",
        "AggIntent::Group",
        "AggIntent::HistogramAvg",
        "AggIntent::HistogramCount",
        "AggIntent::HistogramFraction",
        "AggIntent::HistogramQuantile",
        "AggIntent::HistogramStdDev",
        "AggIntent::HistogramStdVar",
        "AggIntent::HistogramSum",
        "AggIntent::IDelta",
        "AggIntent::IRate",
        "AggIntent::Increase",
        "AggIntent::LastOverTime",
        "AggIntent::MadOverTime",
        "AggIntent::Math",
        "AggIntent::PredictLinear",
        "AggIntent::PresentOverTime",
        "AggIntent::Rate",
        "AggIntent::Resets",
        "AggIntent::TimeFn",
        "AggIntent::TopK",
        "AggIntent::TsOfFirstOverTime",
        "AggIntent::TsOfLastOverTime",
        "AggIntent::TsOfMaxOverTime",
        "AggIntent::TsOfMinOverTime",
    ];

    fn sorted(names: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
        let mut held: Vec<&'static str> = names.into_iter().collect();
        held.sort_unstable();
        held
    }

    #[test]
    fn the_two_tables_name_every_measure_exactly_once() {
        let mut both = sorted(ADMITTED.into_iter().chain(DEFERRED));
        let before = both.len();
        both.dedup();
        assert_eq!(both.len(), before, "a name appears in both tables");
        assert_eq!(
            before, 45,
            "upstream carries 45 measures at 2ec3fc8, not the 44 the plan transcribes"
        );
        assert_eq!(
            both,
            sorted(every_agg_intent().iter().map(|(name, _)| *name))
        );
    }

    #[test]
    fn every_name_in_the_two_tables_is_the_one_intent_name_gives_that_measure() {
        for (name, intent) in every_agg_intent() {
            assert_eq!(intent_name(&intent), name);
        }
    }

    #[test]
    fn every_admitted_measure_lowers_to_an_aggregate_expression() {
        let scope = leaf_scope();
        for (name, intent) in every_agg_intent() {
            if !ADMITTED.contains(&name) {
                continue;
            }
            lower_measure(&intent, &scope, STATE)
                .unwrap_or_else(|refusal| panic!("{name} is admitted: {refusal}"));
        }
    }

    #[test]
    fn every_deferred_measure_is_refused_by_name_with_one_of_the_four_reasons() {
        let scope = leaf_scope();
        for (name, intent) in every_agg_intent() {
            if !DEFERRED.contains(&name) {
                continue;
            }
            let refusal = lower_measure(&intent, &scope, STATE)
                .expect_err("this step translates no such measure");
            assert_eq!(refusal.variant, name);
            assert!(
                crate::df::refusal::RefusalReason::TAGS.contains(&refusal.tag()),
                "{name}: {refusal}"
            );
        }
    }

    #[test]
    fn the_counter_measures_sql_can_reach_are_deferred_as_order_sensitive() {
        let scope = leaf_scope();
        for intent in [AggIntent::Rate, AggIntent::IRate, AggIntent::Increase] {
            let refusal = lower_measure(&intent, &scope, STATE).expect_err("refuses");
            assert_eq!(
                refusal.reason,
                crate::df::refusal::RefusalReason::Deferred {
                    issue: "order-sensitive".to_owned()
                }
            );
        }
    }

    #[test]
    fn a_measure_with_no_column_is_refused_because_the_front_end_always_names_one() {
        let scope = leaf_scope();
        let refusal = lower_measure(&AggIntent::Sum { col: None }, &scope, STATE)
            .expect_err("a column-less Sum has nothing to reduce");
        assert_eq!(refusal.variant, "AggIntent::Sum");
        assert_eq!(refusal.tag(), "no_constructor");
    }

    #[test]
    fn a_count_reduces_every_row_rather_than_a_column() {
        let scope = leaf_scope();
        let lowered = lower_measure(&AggIntent::Count { accuracy: ACCURACY }, &scope, STATE)
            .expect("Count lowers");
        assert!(lowered.readout.is_none());
        assert_eq!(lowered.aggregated, count(lit(1i64)));
    }

    #[test]
    fn a_quantile_ignores_its_accuracy_and_retains_the_whole_column() {
        let scope = leaf_scope();
        let exact = lower_measure(
            &AggIntent::Quantile {
                col: Some(LATENCY),
                q: 0.99,
                accuracy: asap_types::types::AccuracyTarget::Exact,
            },
            &scope,
            STATE,
        )
        .expect("Quantile lowers");
        let approximate = lower_measure(
            &AggIntent::Quantile {
                col: Some(LATENCY),
                q: 0.99,
                accuracy: ACCURACY,
            },
            &scope,
            STATE,
        )
        .expect("Quantile lowers");
        assert_eq!(exact.aggregated, approximate.aggregated);
        assert_eq!(exact.readout, approximate.readout);
        assert!(approximate.readout.is_some());
        let aggregated = exact.aggregated.to_string();
        assert!(aggregated.contains("array_agg"), "{aggregated}");
        let readout = exact
            .readout
            .expect("a quantile reads its column back")
            .to_string();
        assert!(readout.contains("array_sort"), "{readout}");
        assert!(readout.contains("array_element"), "{readout}");
        assert!(
            !format!("{aggregated}{readout}").contains("approx_percentile"),
            "the exact arm never reaches for a t-digest"
        );
    }

    #[test]
    fn a_quantile_outside_the_unit_interval_names_no_position() {
        let scope = leaf_scope();
        let refusal = lower_measure(
            &AggIntent::Quantile {
                col: Some(LATENCY),
                q: 1.5,
                accuracy: ACCURACY,
            },
            &scope,
            STATE,
        )
        .expect_err("refuses");
        assert_eq!(refusal.variant, "AggIntent::Quantile");
        assert_eq!(refusal.tag(), "no_constructor");
    }
}
