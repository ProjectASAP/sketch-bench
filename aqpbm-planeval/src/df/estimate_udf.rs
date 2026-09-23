use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use asap_types::post_asap::{
    PostAsapNodeId, SketchAlgorithm, SketchParams, SketchQuery, SummaryFamilyType,
};
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, Float64Array, Int64Builder, ListBuilder, StringArray,
    StringBuilder, StructBuilder,
};
use datafusion::arrow::datatypes::{DataType as ArrowDataType, Field, Fields};
use datafusion::common::ScalarValue;
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use datafusion::logical_expr::{
    ColumnarValue, Expr, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::prelude::lit;

use crate::df::refusal::Refusal;
use crate::df::sketch_udaf::{
    params_literal, ver_one_refusal, CountMinBinding, CountMinHeapBinding, CountSketchBinding,
    CountSketchHeapBinding, DdSketchBinding, HllBinding, KllBinding, KmvBinding, SketchBinding,
    UnivMonBinding,
};
use crate::handle;
use crate::types::Answer;

pub const KLL_QUANTILE: &str = "asap_estimate_kll_quantile";
pub const DDSKETCH_QUANTILE: &str = "asap_estimate_ddsketch_quantile";
pub const HLL_CARDINALITY: &str = "asap_estimate_hll_cardinality";
pub const KMV_CARDINALITY: &str = "asap_estimate_kmv_cardinality";
pub const UNIVMON_CARDINALITY: &str = "asap_estimate_univmon_cardinality";
pub const CMS_POINT_COUNT: &str = "asap_estimate_cms_point_count";
pub const COUNT_SKETCH_POINT_COUNT: &str = "asap_estimate_count_sketch_point_count";
pub const CMS_WITH_HEAP_POINT_COUNT: &str = "asap_estimate_cms_with_heap_point_count";
pub const COUNT_SKETCH_WITH_HEAP_POINT_COUNT: &str =
    "asap_estimate_count_sketch_with_heap_point_count";
pub const CMS_WITH_HEAP_TOPK: &str = "asap_estimate_cms_with_heap_topk";
pub const COUNT_SKETCH_WITH_HEAP_TOPK: &str = "asap_estimate_count_sketch_with_heap_topk";
pub const UNIVMON_TOPK: &str = "asap_estimate_univmon_topk";
pub const UNIVMON_FREQUENCY_L2: &str = "asap_estimate_univmon_frequency_l2";
pub const UNIVMON_FREQUENCY_ENTROPY: &str = "asap_estimate_univmon_frequency_entropy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadoutShape {
    Scalar,
    Ranked,
}

pub fn ranked_entry_fields() -> Fields {
    Fields::from(vec![
        Field::new("key", ArrowDataType::Utf8, false),
        Field::new("count", ArrowDataType::Int64, false),
    ])
}

pub fn ranked_type() -> ArrowDataType {
    ArrowDataType::List(Arc::new(Field::new(
        "item",
        ArrowDataType::Struct(ranked_entry_fields()),
        true,
    )))
}

impl ReadoutShape {
    pub fn output_type(self) -> ArrowDataType {
        match self {
            ReadoutShape::Scalar => ArrowDataType::Float64,
            ReadoutShape::Ranked => ranked_type(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadoutCall {
    pub function: &'static str,
    pub params_literal: String,
    pub query_literal: String,
    pub shape: ReadoutShape,
}

impl ReadoutCall {
    pub fn arguments(&self, state: Expr) -> Vec<Expr> {
        vec![
            state,
            lit(self.params_literal.clone()),
            lit(self.query_literal.clone()),
        ]
    }

    pub fn output_type(&self) -> ArrowDataType {
        self.shape.output_type()
    }
}

fn does_not_answer(algorithm: &SketchAlgorithm, query: &SketchQuery) -> Refusal {
    Refusal::no_constructor(
        format!("SummaryEstimate({query:?})"),
        format!("{algorithm:?} does not answer that question"),
    )
}

pub fn readout_call(
    family: &SummaryFamilyType,
    query: &SketchQuery,
    node: PostAsapNodeId,
) -> Result<ReadoutCall, Refusal> {
    let SummaryFamilyType::Sketch(kind, _) = family else {
        return Err(Refusal::no_constructor(
            format!("{family:?}"),
            "a readout reads a Sketch state column",
        ));
    };
    handle::check_bindable(family, node).map_err(ver_one_refusal)?;
    let algorithm = kind.algorithm();
    let (function, shape) = readout_function(algorithm, query)?;
    Ok(ReadoutCall {
        function,
        params_literal: params_literal(kind.params())?,
        query_literal: query_literal(query)?,
        shape,
    })
}

pub fn query_literal(query: &SketchQuery) -> Result<String, Refusal> {
    serde_json::to_string(query).map_err(|error| {
        Refusal::no_constructor(
            "SketchQuery",
            format!("the query does not serialize: {error}"),
        )
    })
}

pub fn readout_function(
    algorithm: &SketchAlgorithm,
    query: &SketchQuery,
) -> Result<(&'static str, ReadoutShape), Refusal> {
    match query {
        SketchQuery::Quantile { q } => {
            if !(*q > 0.0 && *q <= 1.0) {
                return Err(Refusal::no_constructor(
                    format!("SketchQuery::Quantile {{ q: {q} }}"),
                    "a quantile rank outside (0, 1] names no order statistic",
                ));
            }
            match algorithm {
                SketchAlgorithm::Kll => Ok((KLL_QUANTILE, ReadoutShape::Scalar)),
                SketchAlgorithm::DDSketch => Ok((DDSKETCH_QUANTILE, ReadoutShape::Scalar)),
                SketchAlgorithm::UnivMon
                | SketchAlgorithm::Cms
                | SketchAlgorithm::Hll
                | SketchAlgorithm::CmsWithHeap
                | SketchAlgorithm::Kmv
                | SketchAlgorithm::Theta
                | SketchAlgorithm::CountSketch
                | SketchAlgorithm::CountSketchWithHeap => Err(does_not_answer(algorithm, query)),
            }
        }
        SketchQuery::Cardinality => match algorithm {
            SketchAlgorithm::Hll => Ok((HLL_CARDINALITY, ReadoutShape::Scalar)),
            SketchAlgorithm::Kmv => Ok((KMV_CARDINALITY, ReadoutShape::Scalar)),
            SketchAlgorithm::UnivMon => Ok((UNIVMON_CARDINALITY, ReadoutShape::Scalar)),
            SketchAlgorithm::Kll
            | SketchAlgorithm::DDSketch
            | SketchAlgorithm::Cms
            | SketchAlgorithm::CmsWithHeap
            | SketchAlgorithm::Theta
            | SketchAlgorithm::CountSketch
            | SketchAlgorithm::CountSketchWithHeap => Err(does_not_answer(algorithm, query)),
        },
        SketchQuery::PointCount { value: None, .. } => Err(Refusal::no_constructor(
            "SketchQuery::PointCount { value: None }",
            "looking up one key per row needs the key relation as a second input, and no EdgeRole \
             carries it",
        )),
        SketchQuery::PointCount { value: Some(_), .. } => match algorithm {
            SketchAlgorithm::Cms => Ok((CMS_POINT_COUNT, ReadoutShape::Scalar)),
            SketchAlgorithm::CountSketch => Ok((COUNT_SKETCH_POINT_COUNT, ReadoutShape::Scalar)),
            SketchAlgorithm::CmsWithHeap => Ok((CMS_WITH_HEAP_POINT_COUNT, ReadoutShape::Scalar)),
            SketchAlgorithm::CountSketchWithHeap => {
                Ok((COUNT_SKETCH_WITH_HEAP_POINT_COUNT, ReadoutShape::Scalar))
            }
            SketchAlgorithm::UnivMon
            | SketchAlgorithm::Kll
            | SketchAlgorithm::Hll
            | SketchAlgorithm::DDSketch
            | SketchAlgorithm::Kmv
            | SketchAlgorithm::Theta => Err(does_not_answer(algorithm, query)),
        },
        SketchQuery::TopK { .. } => match algorithm {
            SketchAlgorithm::CmsWithHeap => Ok((CMS_WITH_HEAP_TOPK, ReadoutShape::Ranked)),
            SketchAlgorithm::CountSketchWithHeap => {
                Ok((COUNT_SKETCH_WITH_HEAP_TOPK, ReadoutShape::Ranked))
            }
            SketchAlgorithm::UnivMon => Ok((UNIVMON_TOPK, ReadoutShape::Ranked)),
            SketchAlgorithm::Kll
            | SketchAlgorithm::Cms
            | SketchAlgorithm::Hll
            | SketchAlgorithm::DDSketch
            | SketchAlgorithm::Kmv
            | SketchAlgorithm::Theta
            | SketchAlgorithm::CountSketch => Err(does_not_answer(algorithm, query)),
        },
        SketchQuery::FrequencyL2 => match algorithm {
            SketchAlgorithm::UnivMon => Ok((UNIVMON_FREQUENCY_L2, ReadoutShape::Scalar)),
            SketchAlgorithm::Kll
            | SketchAlgorithm::Cms
            | SketchAlgorithm::Hll
            | SketchAlgorithm::DDSketch
            | SketchAlgorithm::CmsWithHeap
            | SketchAlgorithm::Kmv
            | SketchAlgorithm::Theta
            | SketchAlgorithm::CountSketch
            | SketchAlgorithm::CountSketchWithHeap => Err(does_not_answer(algorithm, query)),
        },
        SketchQuery::FrequencyEntropy => match algorithm {
            SketchAlgorithm::UnivMon => Ok((UNIVMON_FREQUENCY_ENTROPY, ReadoutShape::Scalar)),
            SketchAlgorithm::Kll
            | SketchAlgorithm::Cms
            | SketchAlgorithm::Hll
            | SketchAlgorithm::DDSketch
            | SketchAlgorithm::CmsWithHeap
            | SketchAlgorithm::Kmv
            | SketchAlgorithm::Theta
            | SketchAlgorithm::CountSketch
            | SketchAlgorithm::CountSketchWithHeap => Err(does_not_answer(algorithm, query)),
        },
    }
}

pub struct SketchReadout<S: SketchBinding> {
    name: &'static str,
    shape: ReadoutShape,
    signature: Signature,
    binding: PhantomData<S>,
}

impl<S: SketchBinding> SketchReadout<S> {
    pub fn new(name: &'static str, shape: ReadoutShape) -> Self {
        Self {
            name,
            shape,
            signature: Signature::exact(
                vec![
                    ArrowDataType::Binary,
                    ArrowDataType::Utf8,
                    ArrowDataType::Utf8,
                ],
                Volatility::Immutable,
            ),
            binding: PhantomData,
        }
    }

    pub fn udf(name: &'static str, shape: ReadoutShape) -> ScalarUDF {
        ScalarUDF::from(Self::new(name, shape))
    }
}

impl<S: SketchBinding> fmt::Debug for SketchReadout<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SketchReadout")
            .field("function", &self.name)
            .field("shape", &self.shape)
            .finish()
    }
}

fn repeated_literal(value: &ColumnarValue, position: &str) -> DataFusionResult<String> {
    match value {
        ColumnarValue::Scalar(ScalarValue::Utf8(Some(held))) => Ok(held.clone()),
        ColumnarValue::Array(array) => {
            let array = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| {
                    DataFusionError::Internal(format!(
                        "the {position} argument reads as Utf8, got {}",
                        array.data_type()
                    ))
                })?;
            let first = (!array.is_empty() && !array.is_null(0))
                .then(|| array.value(0))
                .ok_or_else(|| {
                    DataFusionError::Internal(format!("the {position} argument is null"))
                })?;
            if (1..array.len()).any(|row| array.is_null(row) || array.value(row) != first) {
                return Err(DataFusionError::Internal(format!(
                    "the {position} argument is one literal for the whole projection"
                )));
            }
            Ok(first.to_owned())
        }
        other => Err(DataFusionError::Internal(format!(
            "the {position} argument is a Utf8 literal, got {other:?}"
        ))),
    }
}

fn scalar_answers(answers: Vec<Answer>, function: &str) -> DataFusionResult<ArrayRef> {
    let mut values = Vec::with_capacity(answers.len());
    for answer in answers {
        match answer {
            Answer::Scalar(value) => values.push(value),
            Answer::Ranked(_) => {
                return Err(DataFusionError::Internal(format!(
                    "{function} returns one number per row"
                )))
            }
        }
    }
    Ok(Arc::new(Float64Array::from(values)))
}

pub(crate) fn ranked_answers(answers: Vec<Answer>, function: &str) -> DataFusionResult<ArrayRef> {
    let entries = StructBuilder::new(
        ranked_entry_fields(),
        vec![
            Box::new(StringBuilder::new()),
            Box::new(Int64Builder::new()),
        ],
    );
    let mut lists = ListBuilder::new(entries).with_field(Arc::new(Field::new(
        "item",
        ArrowDataType::Struct(ranked_entry_fields()),
        true,
    )));
    for answer in answers {
        let Answer::Ranked(ranked) = answer else {
            return Err(DataFusionError::Internal(format!(
                "{function} returns a ranked list per row"
            )));
        };
        let entries = lists.values();
        for (key, count) in ranked {
            entries
                .field_builder::<StringBuilder>(0)
                .expect("the first ranked field is the key")
                .append_value(key.rendered());
            entries
                .field_builder::<Int64Builder>(1)
                .expect("the second ranked field is the count")
                .append_value(count as i64);
            entries.append(true);
        }
        lists.append(true);
    }
    Ok(Arc::new(lists.finish()))
}

impl<S: SketchBinding> ScalarUDFImpl for SketchReadout<S> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(self.shape.output_type())
    }

    fn invoke_batch(
        &self,
        args: &[ColumnarValue],
        number_rows: usize,
    ) -> DataFusionResult<ColumnarValue> {
        let [state, params, query] = args else {
            return Err(DataFusionError::Internal(format!(
                "{} takes a state, its parameters and its query, got {} arguments",
                self.name,
                args.len()
            )));
        };
        let params: SketchParams = serde_json::from_str(&repeated_literal(params, "parameters")?)
            .map_err(|error| {
            DataFusionError::Internal(format!("{} cannot read its parameters: {error}", self.name))
        })?;
        let query: SketchQuery =
            serde_json::from_str(&repeated_literal(query, "query")?).map_err(|error| {
                DataFusionError::Internal(format!("{} cannot read its query: {error}", self.name))
            })?;
        let state = state.clone().into_array(number_rows)?;
        let state = state
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| {
                DataFusionError::Internal(format!(
                    "{} reads a Binary state column, got {}",
                    self.name,
                    state.data_type()
                ))
            })?;

        let mut answers = Vec::with_capacity(state.len());
        for row in 0..state.len() {
            if state.is_null(row) {
                return Err(Refusal::deferred(
                    format!("null state at row {row}"),
                    "null-summary-state",
                    "a null state column names no summary to read",
                )
                .into());
            }
            let mut sketch = S::from_bytes(&params, state.value(row))?;
            answers.push(sketch.read(&query)?);
        }

        let array = match self.shape {
            ReadoutShape::Scalar => scalar_answers(answers, self.name)?,
            ReadoutShape::Ranked => ranked_answers(answers, self.name)?,
        };
        Ok(ColumnarValue::Array(array))
    }
}

pub fn readout_scalars() -> Vec<ScalarUDF> {
    vec![
        SketchReadout::<KllBinding>::udf(KLL_QUANTILE, ReadoutShape::Scalar),
        SketchReadout::<DdSketchBinding>::udf(DDSKETCH_QUANTILE, ReadoutShape::Scalar),
        SketchReadout::<HllBinding>::udf(HLL_CARDINALITY, ReadoutShape::Scalar),
        SketchReadout::<KmvBinding>::udf(KMV_CARDINALITY, ReadoutShape::Scalar),
        SketchReadout::<UnivMonBinding>::udf(UNIVMON_CARDINALITY, ReadoutShape::Scalar),
        SketchReadout::<CountMinBinding>::udf(CMS_POINT_COUNT, ReadoutShape::Scalar),
        SketchReadout::<CountSketchBinding>::udf(COUNT_SKETCH_POINT_COUNT, ReadoutShape::Scalar),
        SketchReadout::<CountMinHeapBinding>::udf(CMS_WITH_HEAP_POINT_COUNT, ReadoutShape::Scalar),
        SketchReadout::<CountSketchHeapBinding>::udf(
            COUNT_SKETCH_WITH_HEAP_POINT_COUNT,
            ReadoutShape::Scalar,
        ),
        SketchReadout::<CountMinHeapBinding>::udf(CMS_WITH_HEAP_TOPK, ReadoutShape::Ranked),
        SketchReadout::<CountSketchHeapBinding>::udf(
            COUNT_SKETCH_WITH_HEAP_TOPK,
            ReadoutShape::Ranked,
        ),
        SketchReadout::<UnivMonBinding>::udf(UNIVMON_TOPK, ReadoutShape::Ranked),
        SketchReadout::<UnivMonBinding>::udf(UNIVMON_FREQUENCY_L2, ReadoutShape::Scalar),
        SketchReadout::<UnivMonBinding>::udf(UNIVMON_FREQUENCY_ENTROPY, ReadoutShape::Scalar),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_types::post_asap::{GroupingStrategy, SketchKind};
    use asap_types::pre_asap::ColumnRef;

    const ALGORITHMS: [SketchAlgorithm; 10] = [
        SketchAlgorithm::UnivMon,
        SketchAlgorithm::Kll,
        SketchAlgorithm::Cms,
        SketchAlgorithm::Hll,
        SketchAlgorithm::DDSketch,
        SketchAlgorithm::CmsWithHeap,
        SketchAlgorithm::Kmv,
        SketchAlgorithm::Theta,
        SketchAlgorithm::CountSketch,
        SketchAlgorithm::CountSketchWithHeap,
    ];

    fn queries() -> Vec<SketchQuery> {
        vec![
            SketchQuery::Quantile { q: 0.99 },
            SketchQuery::Cardinality,
            SketchQuery::PointCount {
                key: ColumnRef::Named("user".into()),
                value: Some("u3".into()),
            },
            SketchQuery::PointCount {
                key: ColumnRef::SampleValue,
                value: None,
            },
            SketchQuery::TopK { k: 5 },
            SketchQuery::FrequencyL2,
            SketchQuery::FrequencyEntropy,
        ]
    }

    fn params_for(algorithm: &SketchAlgorithm) -> SketchParams {
        match algorithm {
            SketchAlgorithm::UnivMon => SketchParams::UnivMon {
                heap_size: 16,
                sketch_rows: 4,
                sketch_cols: 2048,
                layers: 8,
            },
            SketchAlgorithm::Kll => SketchParams::Kll { k: 269 },
            SketchAlgorithm::Cms => SketchParams::Cms {
                width: 2048,
                depth: 4,
            },
            SketchAlgorithm::Hll => SketchParams::Hll { precision: 14 },
            SketchAlgorithm::DDSketch => SketchParams::DDSketch { alpha: 0.01 },
            SketchAlgorithm::CmsWithHeap => SketchParams::CmsWithHeap {
                width: 2048,
                depth: 4,
                heap_size: 16,
            },
            SketchAlgorithm::Kmv => SketchParams::Kmv { k: 1024 },
            SketchAlgorithm::Theta => SketchParams::Theta { k: 1024 },
            SketchAlgorithm::CountSketch => SketchParams::CountSketch {
                width: 2048,
                depth: 4,
            },
            SketchAlgorithm::CountSketchWithHeap => SketchParams::CountSketchWithHeap {
                width: 2048,
                depth: 4,
                heap_size: 16,
            },
        }
    }

    #[test]
    fn fourteen_pairings_answer_and_every_other_one_says_why() {
        let mut admitted: Vec<(String, &'static str)> = Vec::new();
        let mut refused = 0usize;
        for algorithm in ALGORITHMS {
            for query in queries() {
                match readout_function(&algorithm, &query) {
                    Ok((function, _)) => admitted.push((format!("{algorithm:?}"), function)),
                    Err(refusal) => {
                        assert_eq!(refusal.tag(), "no_constructor", "{algorithm:?} {query:?}");
                        refused += 1;
                    }
                }
            }
        }
        assert_eq!(admitted.len(), 14);
        assert_eq!(admitted.len() + refused, ALGORITHMS.len() * queries().len());
        let mut functions: Vec<&str> = admitted.iter().map(|(_, function)| *function).collect();
        functions.sort_unstable();
        functions.dedup();
        assert_eq!(functions.len(), 14);
        assert_eq!(readout_scalars().len(), 14);
        let registered: Vec<String> = readout_scalars()
            .iter()
            .map(|function| function.name().to_owned())
            .collect();
        for function in functions {
            assert!(registered.contains(&function.to_owned()), "{function}");
        }
    }

    #[test]
    fn each_query_names_the_families_that_answer_it() {
        let answering = |query: SketchQuery| {
            let mut names: Vec<String> = ALGORITHMS
                .iter()
                .filter(|algorithm| readout_function(algorithm, &query).is_ok())
                .map(|algorithm| format!("{algorithm:?}"))
                .collect();
            names.sort();
            names
        };
        assert_eq!(
            answering(SketchQuery::Quantile { q: 0.5 }),
            vec!["DDSketch", "Kll"]
        );
        assert_eq!(
            answering(SketchQuery::Cardinality),
            vec!["Hll", "Kmv", "UnivMon"]
        );
        assert_eq!(
            answering(SketchQuery::PointCount {
                key: ColumnRef::Named("user".into()),
                value: Some("u3".into())
            }),
            vec!["Cms", "CmsWithHeap", "CountSketch", "CountSketchWithHeap"]
        );
        assert_eq!(
            answering(SketchQuery::TopK { k: 5 }),
            vec!["CmsWithHeap", "CountSketchWithHeap", "UnivMon"]
        );
        assert_eq!(answering(SketchQuery::FrequencyL2), vec!["UnivMon"]);
        assert_eq!(answering(SketchQuery::FrequencyEntropy), vec!["UnivMon"]);
    }

    #[test]
    fn a_point_count_without_a_value_has_no_key_relation_to_read() {
        for algorithm in ALGORITHMS {
            let refused = readout_function(
                &algorithm,
                &SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                },
            )
            .unwrap_err();
            assert_eq!(refused.tag(), "no_constructor");
            assert!(refused.to_string().contains("EdgeRole"), "{refused}");
        }
    }

    #[test]
    fn a_quantile_rank_outside_the_unit_interval_names_no_order_statistic() {
        for q in [-0.1, 0.0, 1.1, f64::NAN] {
            let refused =
                readout_function(&SketchAlgorithm::Kll, &SketchQuery::Quantile { q }).unwrap_err();
            assert_eq!(refused.tag(), "no_constructor");
        }
        assert!(readout_function(&SketchAlgorithm::Kll, &SketchQuery::Quantile { q: 1.0 }).is_ok());
    }

    #[test]
    fn the_output_type_of_each_query_is_the_one_the_scorer_reads() {
        assert_eq!(ReadoutShape::Scalar.output_type(), ArrowDataType::Float64);
        assert_eq!(
            ReadoutShape::Ranked.output_type(),
            ArrowDataType::List(Arc::new(Field::new(
                "item",
                ArrowDataType::Struct(Fields::from(vec![
                    Field::new("key", ArrowDataType::Utf8, false),
                    Field::new("count", ArrowDataType::Int64, false),
                ])),
                true,
            )))
        );
        for function in readout_scalars() {
            let declared = function
                .inner()
                .return_type(&[
                    ArrowDataType::Binary,
                    ArrowDataType::Utf8,
                    ArrowDataType::Utf8,
                ])
                .unwrap();
            let ranked = function.name().ends_with("_topk");
            assert_eq!(
                declared,
                if ranked {
                    ranked_type()
                } else {
                    ArrowDataType::Float64
                },
                "{}",
                function.name()
            );
        }
    }

    #[test]
    fn a_readout_call_carries_the_parameters_and_the_query_it_was_planned_with() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let query = SketchQuery::Quantile { q: 0.99 };
        let call = readout_call(&family, &query, PostAsapNodeId(3)).unwrap();
        assert_eq!(call.function, KLL_QUANTILE);
        assert_eq!(call.shape, ReadoutShape::Scalar);
        assert_eq!(
            serde_json::from_str::<SketchParams>(&call.params_literal).unwrap(),
            SketchParams::Kll { k: 269 }
        );
        assert_eq!(
            serde_json::from_str::<SketchQuery>(&call.query_literal).unwrap(),
            query
        );
        assert_eq!(call.arguments(datafusion::prelude::col("state")).len(), 3);
    }

    #[test]
    fn theta_is_refused_before_a_readout_is_chosen_for_it() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Theta, params_for(&SketchAlgorithm::Theta)),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let refused =
            readout_call(&family, &SketchQuery::Cardinality, PostAsapNodeId(3)).unwrap_err();
        assert_eq!(refused.tag(), "no_constructor");
    }

    #[test]
    fn every_admitted_pairing_has_a_registered_function_of_the_right_shape() {
        let registered = readout_scalars();
        for algorithm in ALGORITHMS {
            for query in queries() {
                let Ok((name, shape)) = readout_function(&algorithm, &query) else {
                    continue;
                };
                let family = SummaryFamilyType::Sketch(
                    SketchKind::new(algorithm.clone(), params_for(&algorithm)),
                    GroupingStrategy::PerSubpopulationInstance,
                );
                let call = readout_call(&family, &query, PostAsapNodeId(3)).unwrap();
                assert_eq!(call.function, name);
                assert_eq!(call.shape, shape);
                let function = registered
                    .iter()
                    .find(|candidate| candidate.name() == name)
                    .unwrap_or_else(|| panic!("{name} is registered"));
                assert_eq!(
                    function
                        .inner()
                        .return_type(&[
                            ArrowDataType::Binary,
                            ArrowDataType::Utf8,
                            ArrowDataType::Utf8
                        ])
                        .unwrap(),
                    shape.output_type()
                );
            }
        }
    }
}
