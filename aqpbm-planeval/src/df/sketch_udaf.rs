use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use asap_sketchlib::common::heap::HHHeap;
use asap_sketchlib::input::{DataInput, HeapItem};
use asap_sketchlib::sketch_framework::univmon::UnivMon;
use asap_sketchlib::sketches::hll::HyperLogLogImpl;
use asap_sketchlib::{
    CMSHeap, CSHeap, Classic, Count, CountMin, DDSketch, FastPath, HllBucketListP12,
    HllBucketListP14, HllBucketListP16, Vector2D, KLL, KMV,
};
use asap_types::post_asap::{
    GroupingStrategy, PostAsapNodeId, SketchAlgorithm, SketchKind, SketchParams, SketchQuery,
    SummaryFamilyType, SummaryInputExpr, SummaryUpdate, WeightDomain,
};
use asap_types::pre_asap::ColumnRef;
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, Float64Array, Int64Array, StringArray,
};
use datafusion::arrow::datatypes::{DataType as ArrowDataType, Field};
use datafusion::common::{Column, ScalarValue, TableReference};
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::utils::format_state_name;
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, Expr, ScalarUDF, Signature, Volatility,
};
use datafusion::physical_expr::expressions::Literal;
use datafusion::physical_expr::PhysicalExpr;
use datafusion::prelude::lit;

use crate::df::estimate_udf::readout_scalars;
use crate::df::refusal::Refusal;
use crate::df::session::SeedBoundFunctions;
use crate::handle;
use crate::types::{Answer, ItemKey};

pub const KLL_FUNCTION: &str = "asap_sketch_kll";
pub const DDSKETCH_FUNCTION: &str = "asap_sketch_ddsketch";
pub const HLL_FUNCTION: &str = "asap_sketch_hll";
pub const CMS_FUNCTION: &str = "asap_sketch_cms";
pub const COUNT_SKETCH_FUNCTION: &str = "asap_sketch_count_sketch";
pub const CMS_WITH_HEAP_FUNCTION: &str = "asap_sketch_cms_with_heap";
pub const COUNT_SKETCH_WITH_HEAP_FUNCTION: &str = "asap_sketch_count_sketch_with_heap";
pub const KMV_FUNCTION: &str = "asap_sketch_kmv";
pub const UNIVMON_FUNCTION: &str = "asap_sketch_univmon";

pub trait SketchBinding: Send + Sync + Sized + 'static {
    const ALGORITHM: SketchAlgorithm;
    const FUNCTION: &'static str;
    const KEYED: bool;

    fn bind(params: &SketchParams, seed: u64) -> Result<Self, Refusal>;
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal>;
    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal>;
    fn to_bytes(&self) -> Result<Vec<u8>, Refusal>;
    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal>;
    fn footprint(&self) -> usize;
    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal>;
}

pub fn sketch_function(algorithm: &SketchAlgorithm) -> Result<&'static str, Refusal> {
    match algorithm {
        SketchAlgorithm::Kll => Ok(KLL_FUNCTION),
        SketchAlgorithm::DDSketch => Ok(DDSKETCH_FUNCTION),
        SketchAlgorithm::Hll => Ok(HLL_FUNCTION),
        SketchAlgorithm::Cms => Ok(CMS_FUNCTION),
        SketchAlgorithm::CountSketch => Ok(COUNT_SKETCH_FUNCTION),
        SketchAlgorithm::CmsWithHeap => Ok(CMS_WITH_HEAP_FUNCTION),
        SketchAlgorithm::CountSketchWithHeap => Ok(COUNT_SKETCH_WITH_HEAP_FUNCTION),
        SketchAlgorithm::Kmv => Ok(KMV_FUNCTION),
        SketchAlgorithm::UnivMon => Ok(UNIVMON_FUNCTION),
        SketchAlgorithm::Theta => Err(Refusal::no_constructor(
            "SketchAlgorithm::Theta",
            "asap_sketchlib has no Theta sketch at all, and answering it with HLL would leave the \
             readout's ResultGuarantee describing an algorithm that did not run",
        )),
    }
}

pub fn keyed_algorithm(algorithm: &SketchAlgorithm) -> Result<bool, Refusal> {
    match algorithm {
        SketchAlgorithm::Kll => Ok(KllBinding::KEYED),
        SketchAlgorithm::DDSketch => Ok(DdSketchBinding::KEYED),
        SketchAlgorithm::Hll => Ok(HllBinding::KEYED),
        SketchAlgorithm::Cms => Ok(CountMinBinding::KEYED),
        SketchAlgorithm::CountSketch => Ok(CountSketchBinding::KEYED),
        SketchAlgorithm::CmsWithHeap => Ok(CountMinHeapBinding::KEYED),
        SketchAlgorithm::CountSketchWithHeap => Ok(CountSketchHeapBinding::KEYED),
        SketchAlgorithm::Kmv => Ok(KmvBinding::KEYED),
        SketchAlgorithm::UnivMon => Ok(UnivMonBinding::KEYED),
        SketchAlgorithm::Theta => Err(sketch_function(algorithm).unwrap_err()),
    }
}

fn counts_in_a_matrix(algorithm: &SketchAlgorithm) -> bool {
    matches!(
        algorithm,
        SketchAlgorithm::Cms
            | SketchAlgorithm::CountSketch
            | SketchAlgorithm::CmsWithHeap
            | SketchAlgorithm::CountSketchWithHeap
    )
}

pub fn ver_one_refusal(refusal: crate::types::Refusal) -> Refusal {
    match refusal {
        crate::types::Refusal::UnsupportedGrouping { detail, .. } => Refusal::deferred(
            "GroupingStrategy::SharedMultiSubpopulation",
            "hydra-layout",
            detail,
        ),
        crate::types::Refusal::InconsistentSketchKind { detail, .. } => {
            Refusal::no_constructor("SketchKind", detail)
        }
        crate::types::Refusal::ParameterOutOfBounds { detail, .. } => {
            Refusal::no_constructor("SketchParams", detail)
        }
        crate::types::Refusal::UnboundFamily { family, reason, .. } => {
            Refusal::no_constructor(format!("{family:?}"), reason)
        }
        other => Refusal::no_constructor("SummaryFamilyType", other.to_string()),
    }
}

fn refuse_unless_parameters_bind(
    algorithm: SketchAlgorithm,
    params: &SketchParams,
) -> Result<(), Refusal> {
    let family = SummaryFamilyType::Sketch(
        SketchKind::new(algorithm, params.clone()),
        GroupingStrategy::PerSubpopulationInstance,
    );
    handle::check_bindable(&family, PostAsapNodeId(0)).map_err(ver_one_refusal)
}

fn mismatched_parameters(algorithm: SketchAlgorithm, params: &SketchParams) -> Refusal {
    Refusal::no_constructor(
        format!("{algorithm:?}"),
        format!("{params:?} belongs to a different algorithm"),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchAggregateCall {
    pub function: &'static str,
    pub arguments: Vec<Expr>,
    pub params: SketchParams,
}

pub fn sketch_aggregate_call(
    family: &SummaryFamilyType,
    update: &SummaryUpdate,
    node: PostAsapNodeId,
) -> Result<SketchAggregateCall, Refusal> {
    let SummaryFamilyType::Sketch(kind, _) = family else {
        return Err(Refusal::no_constructor(
            format!("{family:?}"),
            "sketch_aggregate_call binds only the Sketch family",
        ));
    };
    handle::check_bindable(family, node).map_err(ver_one_refusal)?;
    let algorithm = kind.algorithm();
    let function = sketch_function(algorithm)?;
    let keyed = keyed_algorithm(algorithm)?;

    if counts_in_a_matrix(algorithm)
        && !matches!(update.weight_domain, WeightDomain::NonNegative { .. })
    {
        return Err(Refusal::deferred(
            format!("SummaryAgg({algorithm:?})"),
            "unproven-weight-domain",
            format!(
                "the counters are i32 and one-sided; {:?} carries no non-negativity proof, so the \
                 first signed weight would be discovered at the row that holds it",
                update.weight_domain
            ),
        ));
    }

    let mut arguments = Vec::with_capacity(3);
    match (keyed, update.item.as_ref()) {
        (true, Some(item)) => arguments.push(lower_item(item)?),
        (true, None) => {
            return Err(Refusal::no_constructor(
                format!("SummaryAgg({algorithm:?})"),
                "this family is keyed and `input.item` is absent; reading the weight as the item \
                 would summarize a different column than the plan names",
            ))
        }
        (false, Some(_)) => return Err(Refusal::no_constructor(
            format!("SummaryAgg({algorithm:?})"),
            "this family is keyless and `input.item` is present; the weight is the value being \
                 summarized",
        )),
        (false, None) => {}
    }
    arguments.push(lower_weight(&update.weight)?);
    let params = kind.params().clone();
    arguments.push(lit(params_literal(&params)?));

    Ok(SketchAggregateCall {
        function,
        arguments,
        params,
    })
}

pub fn params_literal(params: &SketchParams) -> Result<String, Refusal> {
    serde_json::to_string(params).map_err(|error| {
        Refusal::no_constructor(
            "SketchParams",
            format!("parameters do not serialize: {error}"),
        )
    })
}

fn lower_item(expr: &SummaryInputExpr) -> Result<Expr, Refusal> {
    match expr {
        SummaryInputExpr::Column(column) => lower_column(column, "item"),
        SummaryInputExpr::Constant(value) => Ok(lit(*value)),
        SummaryInputExpr::Tuple(_) => Err(Refusal::deferred(
            "SummaryInputExpr::Tuple",
            "composite-item-key",
            "a multi-column key has to become one serialized struct before it hashes as one item",
        )),
        SummaryInputExpr::EntityIdentity(_) => Err(Refusal::promql_only(
            "SummaryInputExpr::EntityIdentity",
            "a PromQL label set has no SQL column to read",
        )),
        SummaryInputExpr::ResetAwareCounterDelta { .. } => Err(Refusal::promql_only(
            "SummaryInputExpr::ResetAwareCounterDelta",
            "a counter-reset correction needs the previous sample of the same series",
        )),
    }
}

fn lower_weight(expr: &SummaryInputExpr) -> Result<Expr, Refusal> {
    match expr {
        SummaryInputExpr::Column(column) => lower_column(column, "weight"),
        SummaryInputExpr::Constant(value) => Ok(lit(*value)),
        SummaryInputExpr::Tuple(_) => Err(Refusal::deferred(
            "SummaryInputExpr::Tuple",
            "tuple-weight",
            "a tuple carries no single number to weight an update with",
        )),
        SummaryInputExpr::EntityIdentity(_) => Err(Refusal::promql_only(
            "SummaryInputExpr::EntityIdentity",
            "a PromQL label set has no SQL column to read",
        )),
        SummaryInputExpr::ResetAwareCounterDelta { .. } => Err(Refusal::promql_only(
            "SummaryInputExpr::ResetAwareCounterDelta",
            "a counter-reset correction needs the previous sample of the same series",
        )),
    }
}

fn lower_column(column: &ColumnRef, position: &str) -> Result<Expr, Refusal> {
    match column {
        ColumnRef::Named(name) => Ok(Expr::Column(Column::new_unqualified(name))),
        ColumnRef::Qualified { table, name } => Ok(Expr::Column(Column::new(
            Some(TableReference::bare(table.clone())),
            name,
        ))),
        ColumnRef::SampleValue => Err(Refusal::promql_only(
            format!("ColumnRef::SampleValue ({position})"),
            "the implicit metric sample value names no column of a SQL relation",
        )),
        ColumnRef::Wildcard => Err(Refusal::deferred(
            format!("ColumnRef::Wildcard ({position})"),
            "wildcard-summary-input",
            "no row of the corpus has yet asked what value a wildcard contributes to a summary",
        )),
    }
}

pub struct SketchAccumulator<S: SketchBinding> {
    sketch: S,
    params: SketchParams,
}

impl<S: SketchBinding> SketchAccumulator<S> {
    pub fn new(sketch: S, params: SketchParams) -> Self {
        Self { sketch, params }
    }
}

impl<S: SketchBinding> fmt::Debug for SketchAccumulator<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SketchAccumulator")
            .field("function", &S::FUNCTION)
            .field("params", &self.params)
            .finish()
    }
}

enum ItemColumn<'a> {
    Utf8(&'a StringArray),
    Int64(&'a Int64Array),
    Float64(&'a Float64Array),
}

impl ItemColumn<'_> {
    fn read(array: &ArrayRef) -> DataFusionResult<ItemColumn<'_>> {
        match array.data_type() {
            ArrowDataType::Utf8 => Ok(ItemColumn::Utf8(
                array.as_any().downcast_ref::<StringArray>().unwrap(),
            )),
            ArrowDataType::Int64 => Ok(ItemColumn::Int64(
                array.as_any().downcast_ref::<Int64Array>().unwrap(),
            )),
            ArrowDataType::Float64 => Ok(ItemColumn::Float64(
                array.as_any().downcast_ref::<Float64Array>().unwrap(),
            )),
            other => Err(Refusal::deferred(
                format!("item column {other}"),
                "item-column-type",
                "a summary item reads as Utf8, Int64 or Float64, the three keys asap_sketchlib \
                 hashes natively",
            )
            .into()),
        }
    }

    fn key(&self, row: usize) -> Option<ItemKey> {
        match self {
            ItemColumn::Utf8(array) => {
                (!array.is_null(row)).then(|| ItemKey::Str(array.value(row).to_owned()))
            }
            ItemColumn::Int64(array) => {
                (!array.is_null(row)).then(|| ItemKey::Int(array.value(row)))
            }
            ItemColumn::Float64(array) => {
                (!array.is_null(row)).then(|| ItemKey::Float(array.value(row)))
            }
        }
    }
}

enum WeightColumn<'a> {
    Float64(&'a Float64Array),
    Int64(&'a Int64Array),
}

impl WeightColumn<'_> {
    fn read(array: &ArrayRef) -> DataFusionResult<WeightColumn<'_>> {
        match array.data_type() {
            ArrowDataType::Float64 => Ok(WeightColumn::Float64(
                array.as_any().downcast_ref::<Float64Array>().unwrap(),
            )),
            ArrowDataType::Int64 => Ok(WeightColumn::Int64(
                array.as_any().downcast_ref::<Int64Array>().unwrap(),
            )),
            other => Err(Refusal::deferred(
                format!("weight column {other}"),
                "weight-column-type",
                "a summary weight reads as Float64 or Int64",
            )
            .into()),
        }
    }

    fn weight(&self, row: usize) -> Option<f64> {
        match self {
            WeightColumn::Float64(array) => (!array.is_null(row)).then(|| array.value(row)),
            WeightColumn::Int64(array) => (!array.is_null(row)).then(|| array.value(row) as f64),
        }
    }
}

fn null_input(position: &str, row: usize) -> DataFusionError {
    Refusal::deferred(
        format!("null {position}"),
        "null-summary-input",
        format!(
            "row {row} carries a null {position}; skipping it would leave the summary describing \
             fewer rows than were scanned, and no corpus query has yet said what it should mean"
        ),
    )
    .into()
}

impl<S: SketchBinding> Accumulator for SketchAccumulator<S> {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DataFusionResult<()> {
        let expected = if S::KEYED { 3 } else { 2 };
        if values.len() != expected {
            return Err(DataFusionError::Internal(format!(
                "{} takes {expected} arguments, got {}",
                S::FUNCTION,
                values.len()
            )));
        }
        let (items, weights) = if S::KEYED {
            (Some(ItemColumn::read(&values[0])?), &values[1])
        } else {
            (None, &values[0])
        };
        let weights = WeightColumn::read(weights)?;
        let rows = values[0].len();
        for row in 0..rows {
            let item = match &items {
                Some(column) => Some(column.key(row).ok_or_else(|| null_input("item", row))?),
                None => None,
            };
            let weight = weights
                .weight(row)
                .ok_or_else(|| null_input("weight", row))?;
            self.sketch.update(item.as_ref(), weight)?;
        }
        Ok(())
    }

    fn evaluate(&mut self) -> DataFusionResult<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.sketch.to_bytes()?)))
    }

    fn state(&mut self) -> DataFusionResult<Vec<ScalarValue>> {
        Ok(vec![self.evaluate()?])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DataFusionResult<()> {
        let [states] = states else {
            return Err(DataFusionError::Internal(format!(
                "{} holds one state column, got {}",
                S::FUNCTION,
                states.len()
            )));
        };
        let states = states
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| {
                DataFusionError::Internal(format!(
                    "{} holds its state as Binary, got {}",
                    S::FUNCTION,
                    states.data_type()
                ))
            })?;
        for row in 0..states.len() {
            if states.is_null(row) {
                continue;
            }
            let mut other = S::from_bytes(&self.params, states.value(row))?;
            self.sketch.merge(&mut other)?;
        }
        Ok(())
    }

    fn size(&self) -> usize {
        std::mem::size_of_val(self) + self.sketch.footprint()
    }
}

pub struct SketchAggregate<S: SketchBinding> {
    signature: Signature,
    seed: u64,
    binding: PhantomData<S>,
}

impl<S: SketchBinding> SketchAggregate<S> {
    pub fn new(seed: u64) -> Self {
        Self {
            signature: Signature::any(if S::KEYED { 3 } else { 2 }, Volatility::Immutable),
            seed,
            binding: PhantomData,
        }
    }

    pub fn udf(seed: u64) -> AggregateUDF {
        AggregateUDF::from(Self::new(seed))
    }
}

impl<S: SketchBinding> fmt::Debug for SketchAggregate<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SketchAggregate")
            .field("function", &S::FUNCTION)
            .field("seed", &self.seed)
            .finish()
    }
}

fn params_from_arguments(
    function: &str,
    exprs: &[Arc<dyn PhysicalExpr>],
) -> DataFusionResult<SketchParams> {
    let last = exprs.last().ok_or_else(|| {
        DataFusionError::Internal(format!(
            "{function} takes its parameters as its last argument"
        ))
    })?;
    let literal = last.as_any().downcast_ref::<Literal>().ok_or_else(|| {
        DataFusionError::Internal(format!(
            "{function} takes its parameters as a literal, got {last}"
        ))
    })?;
    let ScalarValue::Utf8(Some(json)) = literal.value() else {
        return Err(DataFusionError::Internal(format!(
            "{function} takes its parameters as a Utf8 literal, got {}",
            literal.value()
        )));
    };
    serde_json::from_str::<SketchParams>(json).map_err(|error| {
        DataFusionError::Internal(format!("{function} cannot read its parameters: {error}"))
    })
}

impl<S: SketchBinding> AggregateUDFImpl for SketchAggregate<S> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        S::FUNCTION
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(ArrowDataType::Binary)
    }

    fn accumulator(&self, args: AccumulatorArgs) -> DataFusionResult<Box<dyn Accumulator>> {
        let params = params_from_arguments(S::FUNCTION, args.exprs)?;
        let sketch = S::bind(&params, self.seed)?;
        Ok(Box::new(SketchAccumulator::<S>::new(sketch, params)))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> DataFusionResult<Vec<Field>> {
        Ok(vec![Field::new(
            format_state_name(args.name, "sketch"),
            ArrowDataType::Binary,
            true,
        )])
    }

    fn equals(&self, other: &dyn AggregateUDFImpl) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => self.seed == other.seed,
            None => false,
        }
    }

    fn hash_value(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        S::FUNCTION.hash(&mut hasher);
        self.seed.hash(&mut hasher);
        hasher.finish()
    }
}

pub fn sketch_aggregates(seed: u64) -> Vec<AggregateUDF> {
    vec![
        SketchAggregate::<KllBinding>::udf(seed),
        SketchAggregate::<DdSketchBinding>::udf(seed),
        SketchAggregate::<HllBinding>::udf(seed),
        SketchAggregate::<CountMinBinding>::udf(seed),
        SketchAggregate::<CountSketchBinding>::udf(seed),
        SketchAggregate::<CountMinHeapBinding>::udf(seed),
        SketchAggregate::<CountSketchHeapBinding>::udf(seed),
        SketchAggregate::<KmvBinding>::udf(seed),
        SketchAggregate::<UnivMonBinding>::udf(seed),
    ]
}

#[derive(Debug, Default)]
pub struct SummaryFunctions;

impl SeedBoundFunctions for SummaryFunctions {
    fn aggregates(&self, seed: u64) -> Vec<AggregateUDF> {
        sketch_aggregates(seed)
    }

    fn scalars(&self, _seed: u64) -> Vec<ScalarUDF> {
        readout_scalars()
    }
}

fn wrong_query(function: &str, query: &SketchQuery) -> Refusal {
    Refusal::no_constructor(
        format!("SummaryEstimate({query:?})"),
        format!("{function} does not answer that question"),
    )
}

fn require_item<'a>(item: Option<&'a ItemKey>, function: &str) -> Result<DataInput<'a>, Refusal> {
    match item {
        Some(ItemKey::Str(held)) => Ok(DataInput::Str(held)),
        Some(ItemKey::Int(held)) => Ok(DataInput::I64(*held)),
        Some(ItemKey::Float(held)) => Ok(DataInput::F64(*held)),
        None => Err(Refusal::no_constructor(
            function,
            "this family is keyed; `input.item` must name a column",
        )),
    }
}

fn reject_item(item: Option<&ItemKey>, function: &str) -> Result<(), Refusal> {
    match item {
        None => Ok(()),
        Some(_) => Err(Refusal::no_constructor(
            function,
            "this family is keyless; `input.item` must be absent",
        )),
    }
}

fn require_unit_weight(weight: f64, function: &str) -> Result<(), Refusal> {
    if weight == 1.0 {
        return Ok(());
    }
    Err(Refusal::no_constructor(
        function,
        format!("asap_sketchlib has no weighted insert here; weight {weight} would be dropped"),
    ))
}

fn require_i32_weight(weight: f64, function: &str) -> Result<i32, Refusal> {
    if weight.is_finite() && weight.fract() == 0.0 && weight >= 0.0 && weight <= f64::from(i32::MAX)
    {
        return Ok(weight as i32);
    }
    Err(Refusal::no_constructor(
        function,
        format!("the counters are i32; weight {weight} is not a non-negative integer they hold"),
    ))
}

fn point_key<'a>(function: &str, query: &'a SketchQuery) -> Result<DataInput<'a>, Refusal> {
    match query {
        SketchQuery::PointCount {
            value: Some(value), ..
        } => Ok(DataInput::Str(value)),
        other => Err(wrong_query(function, other)),
    }
}

fn ranked_from_heap(function: &str, heap: &HHHeap, k: usize) -> Result<Answer, Refusal> {
    let mut ranked: Vec<(ItemKey, u64)> = Vec::with_capacity(heap.len());
    for item in heap.heap() {
        let key = match &item.key {
            HeapItem::String(held) => ItemKey::Str(held.clone()),
            HeapItem::I64(held) => ItemKey::Int(*held),
            HeapItem::F64(held) => ItemKey::Float(*held),
            other => {
                return Err(Refusal::no_constructor(
                    function,
                    format!(
                        "the heap holds a {other:?} key that no update here could have put there"
                    ),
                ))
            }
        };
        ranked.push((key, item.count.max(0) as u64));
    }
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.total_cmp(&right.0))
    });
    ranked.truncate(k);
    Ok(Answer::Ranked(ranked))
}

fn encoding_failed(function: &str, error: impl fmt::Display) -> Refusal {
    Refusal::no_constructor(function, format!("the state does not serialize: {error}"))
}

fn decoding_failed(function: &str, error: impl fmt::Display) -> Refusal {
    Refusal::no_constructor(function, format!("the state does not deserialize: {error}"))
}

pub struct KllBinding {
    inner: KLL<f64>,
    k: u32,
}

impl SketchBinding for KllBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::Kll;
    const FUNCTION: &'static str = KLL_FUNCTION;
    const KEYED: bool = false;

    fn bind(params: &SketchParams, seed: u64) -> Result<Self, Refusal> {
        let SketchParams::Kll { k } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        Ok(Self {
            inner: KLL::<f64>::init_kll_with_seed(*k as i32, seed),
            k: *k,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        reject_item(item, Self::FUNCTION)?;
        self.inner.update(&weight);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::Kll { k } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        Ok(Self {
            inner: KLL::<f64>::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            k: *k,
        })
    }

    fn footprint(&self) -> usize {
        let items =
            handle::kll_max_capacity(self.k as usize, handle::KLL_M) * std::mem::size_of::<f64>();
        let levels = (handle::KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
        let merge_buffer = self.k as usize * std::mem::size_of::<f64>();
        items + levels + merge_buffer
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        match query {
            SketchQuery::Quantile { q } => Ok(Answer::Scalar(self.inner.quantile_cached(*q))),
            other => Err(wrong_query(Self::FUNCTION, other)),
        }
    }
}

pub struct DdSketchBinding {
    inner: DDSketch,
}

impl SketchBinding for DdSketchBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::DDSketch;
    const FUNCTION: &'static str = DDSKETCH_FUNCTION;
    const KEYED: bool = false;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::DDSketch { alpha } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        Ok(Self {
            inner: DDSketch::new(*alpha),
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        reject_item(item, Self::FUNCTION)?;
        if !weight.is_finite() || weight <= 0.0 {
            return Err(Refusal::no_constructor(
                Self::FUNCTION,
                format!(
                    "DDSketch::add drops non-positive and non-finite values with no error channel; \
                     {weight} would leave the summary describing fewer rows than were scanned"
                ),
            ));
        }
        self.inner.add(&weight);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner
            .merge(&other.inner)
            .map_err(|error| Refusal::no_constructor(Self::FUNCTION, error))
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::DDSketch { .. } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        Ok(Self {
            inner: DDSketch::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
        })
    }

    fn footprint(&self) -> usize {
        std::mem::size_of_val(self.inner.store_counts())
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        match query {
            SketchQuery::Quantile { q } => Ok(Answer::Scalar(
                self.inner.get_value_at_quantile(*q).unwrap_or(f64::NAN),
            )),
            other => Err(wrong_query(Self::FUNCTION, other)),
        }
    }
}

pub enum HllBinding {
    P12(HyperLogLogImpl<Classic, HllBucketListP12>),
    P14(HyperLogLogImpl<Classic, HllBucketListP14>),
    P16(HyperLogLogImpl<Classic, HllBucketListP16>),
}

impl SketchBinding for HllBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::Hll;
    const FUNCTION: &'static str = HLL_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::Hll { precision } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        Ok(match precision {
            12 => HllBinding::P12(HyperLogLogImpl::<Classic, HllBucketListP12>::new()),
            14 => HllBinding::P14(HyperLogLogImpl::<Classic, HllBucketListP14>::new()),
            _ => HllBinding::P16(HyperLogLogImpl::<Classic, HllBucketListP16>::new()),
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, _weight: f64) -> Result<(), Refusal> {
        let key = require_item(item, Self::FUNCTION)?;
        match self {
            HllBinding::P12(inner) => inner.insert(&key),
            HllBinding::P14(inner) => inner.insert(&key),
            HllBinding::P16(inner) => inner.insert(&key),
        }
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        match (self, other) {
            (HllBinding::P12(inner), HllBinding::P12(other)) => inner.merge(other),
            (HllBinding::P14(inner), HllBinding::P14(other)) => inner.merge(other),
            (HllBinding::P16(inner), HllBinding::P16(other)) => inner.merge(other),
            _ => {
                return Err(Refusal::no_constructor(
                    Self::FUNCTION,
                    "two states at different precisions hold different register counts",
                ))
            }
        }
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        match self {
            HllBinding::P12(inner) => inner.serialize_to_bytes(),
            HllBinding::P14(inner) => inner.serialize_to_bytes(),
            HllBinding::P16(inner) => inner.serialize_to_bytes(),
        }
        .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::Hll { precision } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        let decode = |error| decoding_failed(Self::FUNCTION, error);
        Ok(match precision {
            12 => HllBinding::P12(
                HyperLogLogImpl::<Classic, HllBucketListP12>::deserialize_from_bytes(bytes)
                    .map_err(decode)?,
            ),
            14 => HllBinding::P14(
                HyperLogLogImpl::<Classic, HllBucketListP14>::deserialize_from_bytes(bytes)
                    .map_err(decode)?,
            ),
            _ => HllBinding::P16(
                HyperLogLogImpl::<Classic, HllBucketListP16>::deserialize_from_bytes(bytes)
                    .map_err(decode)?,
            ),
        })
    }

    fn footprint(&self) -> usize {
        match self {
            HllBinding::P12(_) => HllBucketListP12::NUM_REGISTERS,
            HllBinding::P14(_) => HllBucketListP14::NUM_REGISTERS,
            HllBinding::P16(_) => HllBucketListP16::NUM_REGISTERS,
        }
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(match self {
                HllBinding::P12(inner) => inner.estimate() as f64,
                HllBinding::P14(inner) => inner.estimate() as f64,
                HllBinding::P16(inner) => inner.estimate() as f64,
            })),
            other => Err(wrong_query(Self::FUNCTION, other)),
        }
    }
}

pub struct CountMinBinding {
    inner: CountMin<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    ingested: i64,
}

impl CountMinBinding {
    fn ingested_weight(&self) -> i64 {
        let counts = self.inner.as_storage();
        (0..self.cols)
            .map(|col| i64::from(counts.query_one_counter(0, col)))
            .sum()
    }
}

impl SketchBinding for CountMinBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::Cms;
    const FUNCTION: &'static str = CMS_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::Cms { width, depth } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let (rows, cols) = handle::transposed(*width, *depth);
        Ok(Self {
            inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(rows, cols),
            rows,
            cols,
            ingested: 0,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        let many = require_i32_weight(weight, Self::FUNCTION)?;
        let key = require_item(item, Self::FUNCTION)?;
        let next = self.ingested + i64::from(many);
        if next > i64::from(i32::MAX) {
            return Err(Refusal::no_constructor(
                Self::FUNCTION,
                format!(
                    "{} already ingested plus weight {weight} would exceed i32::MAX = {}, and a \
                     single hot key can land the whole total in one counter",
                    self.ingested,
                    i32::MAX
                ),
            ));
        }
        self.inner.insert_many(&key, many);
        self.ingested = next;
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        self.ingested = self.ingested_weight();
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::Cms { width, depth } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        let (rows, cols) = handle::transposed(*width, *depth);
        let mut bound = Self {
            inner: CountMin::<Vector2D<i32>, FastPath>::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            rows,
            cols,
            ingested: 0,
        };
        bound.ingested = bound.ingested_weight();
        Ok(bound)
    }

    fn footprint(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        let key = point_key(Self::FUNCTION, query)?;
        Ok(Answer::Scalar(f64::from(self.inner.estimate(&key))))
    }
}

pub struct CountSketchBinding {
    inner: Count<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

impl SketchBinding for CountSketchBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::CountSketch;
    const FUNCTION: &'static str = COUNT_SKETCH_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::CountSketch { width, depth } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let (rows, cols) = handle::transposed(*width, *depth);
        Ok(Self {
            inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(rows, cols),
            rows,
            cols,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        let many = require_i32_weight(weight, Self::FUNCTION)?;
        let key = require_item(item, Self::FUNCTION)?;
        self.inner.insert_many(&key, many);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::CountSketch { width, depth } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        let (rows, cols) = handle::transposed(*width, *depth);
        Ok(Self {
            inner: Count::<Vector2D<i32>, FastPath>::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            rows,
            cols,
        })
    }

    fn footprint(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        let key = point_key(Self::FUNCTION, query)?;
        Ok(Answer::Scalar(self.inner.estimate(&key)))
    }
}

pub struct CountMinHeapBinding {
    inner: CMSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    heap_size: usize,
}

impl SketchBinding for CountMinHeapBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::CmsWithHeap;
    const FUNCTION: &'static str = CMS_WITH_HEAP_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::CmsWithHeap {
            width,
            depth,
            heap_size,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let (rows, cols) = handle::transposed(*width, *depth);
        let heap_size = *heap_size as usize;
        Ok(Self {
            inner: CMSHeap::<Vector2D<i32>, FastPath>::new(rows, cols, heap_size),
            rows,
            cols,
            heap_size,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        require_unit_weight(weight, Self::FUNCTION)?;
        let key = require_item(item, Self::FUNCTION)?;
        self.inner.insert(&key);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::CmsWithHeap {
            width,
            depth,
            heap_size,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        let (rows, cols) = handle::transposed(*width, *depth);
        Ok(Self {
            inner: CMSHeap::<Vector2D<i32>, FastPath>::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            rows,
            cols,
            heap_size: *heap_size as usize,
        })
    }

    fn footprint(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
            + handle::heap_bytes(self.inner.heap(), self.heap_size)
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        if let SketchQuery::TopK { k } = query {
            return ranked_from_heap(Self::FUNCTION, self.inner.heap(), *k);
        }
        let key = point_key(Self::FUNCTION, query)?;
        Ok(Answer::Scalar(f64::from(self.inner.estimate(&key))))
    }
}

pub struct CountSketchHeapBinding {
    inner: CSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    heap_size: usize,
}

impl SketchBinding for CountSketchHeapBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::CountSketchWithHeap;
    const FUNCTION: &'static str = COUNT_SKETCH_WITH_HEAP_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::CountSketchWithHeap {
            width,
            depth,
            heap_size,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let (rows, cols) = handle::transposed(*width, *depth);
        let heap_size = *heap_size as usize;
        Ok(Self {
            inner: CSHeap::<Vector2D<i32>, FastPath>::new(rows, cols, heap_size),
            rows,
            cols,
            heap_size,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        require_unit_weight(weight, Self::FUNCTION)?;
        let key = require_item(item, Self::FUNCTION)?;
        self.inner.insert(&key);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::CountSketchWithHeap {
            width,
            depth,
            heap_size,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        let (rows, cols) = handle::transposed(*width, *depth);
        Ok(Self {
            inner: CSHeap::<Vector2D<i32>, FastPath>::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            rows,
            cols,
            heap_size: *heap_size as usize,
        })
    }

    fn footprint(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
            + handle::heap_bytes(self.inner.heap(), self.heap_size)
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        if let SketchQuery::TopK { k } = query {
            return ranked_from_heap(Self::FUNCTION, self.inner.heap(), *k);
        }
        let key = point_key(Self::FUNCTION, query)?;
        Ok(Answer::Scalar(self.inner.estimate(&key)))
    }
}

pub struct KmvBinding {
    inner: KMV,
    k: usize,
}

impl SketchBinding for KmvBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::Kmv;
    const FUNCTION: &'static str = KMV_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::Kmv { k } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let k = *k as usize;
        Ok(Self {
            inner: KMV::new(k),
            k,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, _weight: f64) -> Result<(), Refusal> {
        let key = require_item(item, Self::FUNCTION)?;
        self.inner.insert(&key);
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&mut other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::Kmv { k } = params else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        Ok(Self {
            inner: KMV::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            k: *k as usize,
        })
    }

    fn footprint(&self) -> usize {
        let capacity = handle::grown_capacity(
            self.k.min(handle::SKETCHLIB_PREALLOCATED_SLOTS),
            self.inner.k_vals.len(),
        );
        capacity * std::mem::size_of::<u64>()
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(self.inner.estimate())),
            other => Err(wrong_query(Self::FUNCTION, other)),
        }
    }
}

pub struct UnivMonBinding {
    inner: UnivMon,
    heap_size: usize,
    rows: usize,
    cols: usize,
    layers: usize,
}

impl SketchBinding for UnivMonBinding {
    const ALGORITHM: SketchAlgorithm = SketchAlgorithm::UnivMon;
    const FUNCTION: &'static str = UNIVMON_FUNCTION;
    const KEYED: bool = true;

    fn bind(params: &SketchParams, _seed: u64) -> Result<Self, Refusal> {
        let SketchParams::UnivMon {
            heap_size,
            sketch_rows,
            sketch_cols,
            layers,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        refuse_unless_parameters_bind(Self::ALGORITHM, params)?;
        let (heap_size, rows, cols, layers) = (
            *heap_size as usize,
            *sketch_rows as usize,
            *sketch_cols as usize,
            *layers as usize,
        );
        Ok(Self {
            inner: UnivMon::init_univmon(heap_size, rows, cols, layers),
            heap_size,
            rows,
            cols,
            layers,
        })
    }

    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), Refusal> {
        let count = require_i32_weight(weight, Self::FUNCTION)?;
        let key = require_item(item, Self::FUNCTION)?;
        self.inner.insert(&key, i64::from(count));
        Ok(())
    }

    fn merge(&mut self, other: &mut Self) -> Result<(), Refusal> {
        self.inner.merge(&other.inner);
        Ok(())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Refusal> {
        self.inner
            .serialize_to_bytes()
            .map_err(|error| encoding_failed(Self::FUNCTION, error))
    }

    fn from_bytes(params: &SketchParams, bytes: &[u8]) -> Result<Self, Refusal> {
        let SketchParams::UnivMon {
            heap_size,
            sketch_rows,
            sketch_cols,
            layers,
        } = params
        else {
            return Err(mismatched_parameters(Self::ALGORITHM, params));
        };
        Ok(Self {
            inner: UnivMon::deserialize_from_bytes(bytes)
                .map_err(|error| decoding_failed(Self::FUNCTION, error))?,
            heap_size: *heap_size as usize,
            rows: *sketch_rows as usize,
            cols: *sketch_cols as usize,
            layers: *layers as usize,
        })
    }

    fn footprint(&self) -> usize {
        let counters = (self.rows * self.cols + self.rows) * std::mem::size_of::<i64>();
        let heaps: usize = self
            .inner
            .hh_layers
            .iter()
            .map(|heap| handle::heap_bytes(heap, self.heap_size))
            .sum();
        self.layers * counters + heaps + self.layers
    }

    fn read(&mut self, query: &SketchQuery) -> Result<Answer, Refusal> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(self.inner.calc_card())),
            SketchQuery::FrequencyL2 => Ok(Answer::Scalar(self.inner.calc_l2())),
            SketchQuery::FrequencyEntropy => Ok(Answer::Scalar(self.inner.calc_entropy())),
            SketchQuery::TopK { k } => {
                let k = *k;
                let heap = self.inner.heap_at_layer(0).clone();
                ranked_from_heap(Self::FUNCTION, &heap, k)
            }
            other => Err(wrong_query(Self::FUNCTION, other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_types::post_asap::{NonNegativeWeightProof, SketchKind};
    use datafusion::arrow::datatypes::Schema as ArrowSchema;
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::catalog::TableProvider;
    use datafusion::datasource::{provider_as_source, MemTable};
    use datafusion::logical_expr::LogicalPlanBuilder;
    use datafusion::physical_plan::{collect, displayable};
    use datafusion::prelude::col;

    use crate::df::session::{MemoryPoolSettings, SeedSession};

    const SEED: u64 = 7;

    fn family(algorithm: SketchAlgorithm, params: SketchParams) -> SummaryFamilyType {
        SummaryFamilyType::Sketch(
            SketchKind::new(algorithm, params),
            GroupingStrategy::PerSubpopulationInstance,
        )
    }

    fn params_of(family: &SummaryFamilyType) -> SketchParams {
        match family {
            SummaryFamilyType::Sketch(kind, _) => kind.params().clone(),
            other => panic!("{other:?} is not a sketch"),
        }
    }

    fn keyless_rows() -> Vec<(Option<ItemKey>, f64)> {
        (0..2000)
            .map(|row: i64| (None, ((row * 7919) % 1000) as f64 + 0.5))
            .collect()
    }

    fn keyed_rows() -> Vec<(Option<ItemKey>, f64)> {
        (0..2000)
            .map(|row: i64| (Some(ItemKey::Str(format!("u{}", row % 37))), 1.0))
            .collect()
    }

    fn same_answer(left: &Answer, right: &Answer) -> bool {
        match (left, right) {
            (Answer::Scalar(left), Answer::Scalar(right)) => left.to_bits() == right.to_bits(),
            (Answer::Ranked(left), Answer::Ranked(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|((lk, lc), (rk, rc))| lk.total_cmp(rk).is_eq() && lc == rc)
            }
            _ => false,
        }
    }

    fn ver_one_answer(
        family: &SummaryFamilyType,
        rows: &[(Option<ItemKey>, f64)],
        query: &SketchQuery,
    ) -> Answer {
        let mut bound = handle::bind(family, PostAsapNodeId(1), SEED).expect("ver 1 binds");
        for (item, weight) in rows {
            bound.update(item.as_ref(), *weight).expect("ver 1 update");
        }
        bound.estimate(query).expect("ver 1 readout")
    }

    fn ver_two_answer<S: SketchBinding>(
        params: &SketchParams,
        rows: &[(Option<ItemKey>, f64)],
        query: &SketchQuery,
    ) -> Answer {
        let mut sketch = S::bind(params, SEED).expect("ver 2 binds");
        for (item, weight) in rows {
            sketch.update(item.as_ref(), *weight).expect("ver 2 update");
        }
        let bytes = sketch.to_bytes().expect("ver 2 serializes");
        let mut restored = S::from_bytes(params, &bytes).expect("ver 2 deserializes");
        restored.read(query).expect("ver 2 readout")
    }

    fn agrees_with_ver_one<S: SketchBinding>(
        family: &SummaryFamilyType,
        rows: &[(Option<ItemKey>, f64)],
        queries: &[SketchQuery],
    ) {
        let params = params_of(family);
        for query in queries {
            let ver_one = ver_one_answer(family, rows, query);
            let ver_two = ver_two_answer::<S>(&params, rows, query);
            assert!(
                same_answer(&ver_one, &ver_two),
                "{:?} on {query:?}: ver 1 {ver_one:?} vs ver 2 {ver_two:?}",
                S::ALGORITHM
            );
        }
    }

    fn point_count(value: &str) -> SketchQuery {
        SketchQuery::PointCount {
            key: ColumnRef::Named("user".into()),
            value: Some(value.into()),
        }
    }

    #[test]
    fn kll_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<KllBinding>(
            &family(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
            &keyless_rows(),
            &[
                SketchQuery::Quantile { q: 0.5 },
                SketchQuery::Quantile { q: 0.99 },
                SketchQuery::Quantile { q: 1.0 },
            ],
        );
    }

    #[test]
    fn ddsketch_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<DdSketchBinding>(
            &family(
                SketchAlgorithm::DDSketch,
                SketchParams::DDSketch { alpha: 0.01 },
            ),
            &keyless_rows(),
            &[
                SketchQuery::Quantile { q: 0.5 },
                SketchQuery::Quantile { q: 0.99 },
            ],
        );
    }

    #[test]
    fn hll_reads_out_what_ver_one_reads_out() {
        for precision in [12u8, 14, 16] {
            agrees_with_ver_one::<HllBinding>(
                &family(SketchAlgorithm::Hll, SketchParams::Hll { precision }),
                &keyed_rows(),
                &[SketchQuery::Cardinality],
            );
        }
    }

    #[test]
    fn kmv_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<KmvBinding>(
            &family(SketchAlgorithm::Kmv, SketchParams::Kmv { k: 1024 }),
            &keyed_rows(),
            &[SketchQuery::Cardinality],
        );
    }

    #[test]
    fn cms_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<CountMinBinding>(
            &family(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 2048,
                    depth: 4,
                },
            ),
            &keyed_rows(),
            &[point_count("u3"), point_count("u36"), point_count("absent")],
        );
    }

    #[test]
    fn count_sketch_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<CountSketchBinding>(
            &family(
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch {
                    width: 2048,
                    depth: 4,
                },
            ),
            &keyed_rows(),
            &[point_count("u3"), point_count("u36")],
        );
    }

    #[test]
    fn cms_with_heap_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<CountMinHeapBinding>(
            &family(
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap {
                    width: 2048,
                    depth: 4,
                    heap_size: 16,
                },
            ),
            &keyed_rows(),
            &[point_count("u3"), SketchQuery::TopK { k: 5 }],
        );
    }

    #[test]
    fn count_sketch_with_heap_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<CountSketchHeapBinding>(
            &family(
                SketchAlgorithm::CountSketchWithHeap,
                SketchParams::CountSketchWithHeap {
                    width: 2048,
                    depth: 4,
                    heap_size: 16,
                },
            ),
            &keyed_rows(),
            &[point_count("u3"), SketchQuery::TopK { k: 5 }],
        );
    }

    fn univmon_params() -> SketchParams {
        SketchParams::UnivMon {
            heap_size: 16,
            sketch_rows: 4,
            sketch_cols: 2048,
            layers: 8,
        }
    }

    #[test]
    fn univmon_reads_out_what_ver_one_reads_out() {
        agrees_with_ver_one::<UnivMonBinding>(
            &family(SketchAlgorithm::UnivMon, univmon_params()),
            &keyed_rows(),
            &[SketchQuery::Cardinality, SketchQuery::FrequencyL2],
        );
    }

    fn steps_apart(left: f64, right: f64) -> i128 {
        fn ordered(value: f64) -> i128 {
            let bits = i128::from(value.to_bits() as i64);
            if bits < 0 {
                i128::from(i64::MIN) - bits
            } else {
                bits
            }
        }
        assert!(left.is_finite() && right.is_finite(), "{left} and {right}");
        (ordered(left) - ordered(right)).abs()
    }

    fn one_heavy_key_rows() -> Vec<(Option<ItemKey>, f64)> {
        (0..300)
            .map(|row: i64| {
                let key = if row % 3 == 0 { 0 } else { row % 17 };
                (Some(ItemKey::Str(format!("u{key}"))), 1.0)
            })
            .collect()
    }

    fn many_key_rows() -> Vec<(Option<ItemKey>, f64)> {
        (0..2000)
            .map(|row: i64| (Some(ItemKey::Str(format!("u{}", (row * row) % 211))), 1.0))
            .collect()
    }

    #[test]
    fn univmon_entropy_stays_within_one_step_of_ver_one_across_the_state_round_trip() {
        let params = univmon_params();
        let bound = family(SketchAlgorithm::UnivMon, params.clone());
        let query = SketchQuery::FrequencyEntropy;
        let mut diverged = Vec::new();

        for (name, rows) in [
            ("37 even keys", keyed_rows()),
            ("one heavy key of 17", one_heavy_key_rows()),
            ("211 keys, uneven", many_key_rows()),
        ] {
            let mut direct = UnivMonBinding::bind(&params, SEED).unwrap();
            for (item, weight) in &rows {
                direct.update(item.as_ref(), *weight).unwrap();
            }
            let in_process = direct.read(&query).unwrap();
            let ver_one = ver_one_answer(&bound, &rows, &query);
            assert!(
                same_answer(&in_process, &ver_one),
                "{name}: before serialization {in_process:?} vs ver 1 {ver_one:?}"
            );

            let bytes = direct.to_bytes().unwrap();
            let mut restored = UnivMonBinding::from_bytes(&params, &bytes).unwrap();
            assert_eq!(bytes, restored.to_bytes().unwrap(), "{name}");
            let (Answer::Scalar(through_bytes), Answer::Scalar(expected)) =
                (restored.read(&query).unwrap(), ver_one)
            else {
                panic!("an entropy readout is a scalar");
            };
            let steps = steps_apart(through_bytes, expected);
            assert!(
                steps <= 1,
                "{name}: {through_bytes} and {expected} are {steps} representable values apart, \
                 not one unit in the last place"
            );
            if steps == 1 {
                diverged.push(name);
            }

            for stable in [SketchQuery::Cardinality, SketchQuery::FrequencyL2] {
                assert!(
                    same_answer(
                        &restored.read(&stable).unwrap(),
                        &direct.read(&stable).unwrap()
                    ),
                    "{name}: {stable:?}"
                );
            }
        }

        assert!(
            diverged.contains(&"37 even keys"),
            "the round trip still moves the entropy of the input this bound was read off"
        );
    }

    #[test]
    fn one_step_apart_is_the_next_representable_value_and_nothing_wider() {
        let value = 6.607_797_054_0_f64;
        assert_eq!(steps_apart(value, value), 0);
        assert_eq!(steps_apart(value, f64::from_bits(value.to_bits() + 1)), 1);
        assert_eq!(steps_apart(value, f64::from_bits(value.to_bits() + 2)), 2);
        assert_eq!(steps_apart(2.0, f64::from_bits(2.0_f64.to_bits() - 1)), 1);
        assert_eq!(steps_apart(0.0, -0.0), 0);
        assert_eq!(
            steps_apart(1e300, f64::from_bits(1e300_f64.to_bits() + 1)),
            1
        );
    }

    #[test]
    fn univmon_ranks_its_bottom_layer_and_the_round_trip_keeps_the_ranking() {
        let params = univmon_params();
        let rows = keyed_rows();
        let query = SketchQuery::TopK { k: 5 };
        let mut direct = UnivMonBinding::bind(&params, SEED).unwrap();
        for (item, weight) in &rows {
            direct.update(item.as_ref(), *weight).unwrap();
        }
        let before = direct.read(&query).unwrap();
        let after = ver_two_answer::<UnivMonBinding>(&params, &rows, &query);
        assert!(same_answer(&before, &after), "{before:?} vs {after:?}");
        let Answer::Ranked(ranked) = before else {
            panic!("a top-k readout is ranked");
        };
        assert_eq!(ranked.len(), 5);
        assert!(ranked.windows(2).all(|pair| pair[0].1 >= pair[1].1));
    }

    #[test]
    fn theta_names_the_missing_constructor_and_offers_no_substitute() {
        let refused = sketch_function(&SketchAlgorithm::Theta).unwrap_err();
        assert_eq!(refused.tag(), "no_constructor");
        assert!(refused.detail.contains("HLL"), "{refused}");
        assert!(keyed_algorithm(&SketchAlgorithm::Theta).is_err());
    }

    #[test]
    fn nine_algorithms_name_nine_distinct_functions() {
        let named: Vec<&str> = [
            SketchAlgorithm::Kll,
            SketchAlgorithm::DDSketch,
            SketchAlgorithm::Hll,
            SketchAlgorithm::Cms,
            SketchAlgorithm::CountSketch,
            SketchAlgorithm::CmsWithHeap,
            SketchAlgorithm::CountSketchWithHeap,
            SketchAlgorithm::Kmv,
            SketchAlgorithm::UnivMon,
        ]
        .iter()
        .map(|algorithm| sketch_function(algorithm).unwrap())
        .collect();
        let mut sorted = named.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 9);
        let registered: Vec<String> = sketch_aggregates(SEED)
            .iter()
            .map(|function| function.name().to_owned())
            .collect();
        assert_eq!(registered.len(), 9);
        for name in named {
            assert!(registered.contains(&name.to_owned()), "{name}");
        }
    }

    #[test]
    fn keyless_families_want_the_item_absent_and_keyed_families_want_it_present() {
        let keyless = family(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 });
        let keyed = family(SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 });
        let with_item = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            weight: SummaryInputExpr::Constant(1.0),
            weight_domain: WeightDomain::NonNegative {
                proof: NonNegativeWeightProof::UnitCount,
            },
        };
        let without_item = SummaryUpdate {
            item: None,
            weight: SummaryInputExpr::Column(ColumnRef::Named("latency".into())),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        let node = PostAsapNodeId(1);
        assert!(sketch_aggregate_call(&keyless, &without_item, node).is_ok());
        assert!(sketch_aggregate_call(&keyed, &with_item, node).is_ok());
        assert_eq!(
            sketch_aggregate_call(&keyless, &with_item, node)
                .unwrap_err()
                .tag(),
            "no_constructor"
        );
        assert_eq!(
            sketch_aggregate_call(&keyed, &without_item, node)
                .unwrap_err()
                .tag(),
            "no_constructor"
        );
    }

    #[test]
    fn a_matrix_family_refuses_an_unproven_weight_domain_before_the_first_row() {
        let node = PostAsapNodeId(1);
        let unproven = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            weight: SummaryInputExpr::Constant(1.0),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        let proven = SummaryUpdate {
            weight_domain: WeightDomain::NonNegative {
                proof: NonNegativeWeightProof::UnitCount,
            },
            ..unproven.clone()
        };
        let matrices = [
            (
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 2048,
                    depth: 4,
                },
            ),
            (
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch {
                    width: 2048,
                    depth: 4,
                },
            ),
            (
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap {
                    width: 2048,
                    depth: 4,
                    heap_size: 16,
                },
            ),
            (
                SketchAlgorithm::CountSketchWithHeap,
                SketchParams::CountSketchWithHeap {
                    width: 2048,
                    depth: 4,
                    heap_size: 16,
                },
            ),
        ];
        for (algorithm, params) in matrices {
            let matrix = family(algorithm.clone(), params);
            let refused = sketch_aggregate_call(&matrix, &unproven, node).unwrap_err();
            assert_eq!(refused.tag(), "deferred", "{algorithm:?}");
            assert!(refused.to_string().contains("unproven-weight-domain"));
            assert!(sketch_aggregate_call(&matrix, &proven, node).is_ok());
        }
        let quantile = family(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 });
        let unproven_keyless = SummaryUpdate {
            item: None,
            weight: SummaryInputExpr::Column(ColumnRef::Named("latency".into())),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        assert!(sketch_aggregate_call(&quantile, &unproven_keyless, node).is_ok());
    }

    #[test]
    fn a_summary_input_is_a_column_or_a_constant_and_everything_else_says_why() {
        let label_set = asap_types::post_asap::EntityIdentity::PromqlLabelSet { excluding: vec![] };
        assert!(lower_item(&SummaryInputExpr::Column(ColumnRef::Named("user".into()))).is_ok());
        assert!(lower_item(&SummaryInputExpr::Constant(1.0)).is_ok());
        assert!(lower_item(&SummaryInputExpr::Column(ColumnRef::Qualified {
            table: "t".into(),
            name: "user".into()
        }))
        .is_ok());
        assert_eq!(
            lower_item(&SummaryInputExpr::Tuple(vec![]))
                .unwrap_err()
                .tag(),
            "deferred"
        );
        assert_eq!(
            lower_item(&SummaryInputExpr::EntityIdentity(label_set.clone()))
                .unwrap_err()
                .tag(),
            "promql_only"
        );
        assert_eq!(
            lower_item(&SummaryInputExpr::ResetAwareCounterDelta {
                value: ColumnRef::SampleValue,
                series: label_set,
            })
            .unwrap_err()
            .tag(),
            "promql_only"
        );
        assert_eq!(
            lower_weight(&SummaryInputExpr::Tuple(vec![]))
                .unwrap_err()
                .tag(),
            "deferred"
        );
        assert_eq!(
            lower_weight(&SummaryInputExpr::Column(ColumnRef::SampleValue))
                .unwrap_err()
                .tag(),
            "promql_only"
        );
        assert_eq!(
            lower_weight(&SummaryInputExpr::Column(ColumnRef::Wildcard))
                .unwrap_err()
                .tag(),
            "deferred"
        );
    }

    #[test]
    fn a_hydra_layout_is_deferred_and_a_clamped_parameter_has_no_constructor() {
        let node = PostAsapNodeId(1);
        let update = SummaryUpdate {
            item: None,
            weight: SummaryInputExpr::Column(ColumnRef::Named("latency".into())),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        let over_the_ceiling = family(SketchAlgorithm::Kll, SketchParams::Kll { k: 26_603 });
        let refused = sketch_aggregate_call(&over_the_ceiling, &update, node).unwrap_err();
        assert_eq!(refused.tag(), "no_constructor");
        assert!(refused.to_string().contains("26602"), "{refused}");
        let odd_precision = family(SketchAlgorithm::Hll, SketchParams::Hll { precision: 13 });
        let keyed_update = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            ..update.clone()
        };
        assert_eq!(
            sketch_aggregate_call(&odd_precision, &keyed_update, node)
                .unwrap_err()
                .tag(),
            "no_constructor"
        );
        let hydra = SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
            GroupingStrategy::SharedMultiSubpopulation {
                kind: asap_types::post_asap::HydraKind::HydraKll,
                params: asap_types::post_asap::HydraParams::HydraKll {
                    k: 269,
                    shared_buckets: 1024,
                },
            },
        );
        assert_eq!(
            sketch_aggregate_call(&hydra, &update, node)
                .unwrap_err()
                .tag(),
            "deferred"
        );
    }

    fn refuses_a_foreign_parameter_literal<S: SketchBinding>(params: SketchParams) {
        let sketch = S::bind(&params, SEED).expect("the family binds");
        let bytes = sketch.to_bytes().expect("the state serializes");
        S::from_bytes(&params, &bytes).expect("its own parameters read the state back");
        let foreign = if matches!(params, SketchParams::Kll { .. }) {
            SketchParams::DDSketch { alpha: 0.01 }
        } else {
            SketchParams::Kll { k: 269 }
        };
        let Err(refused) = S::from_bytes(&foreign, &bytes) else {
            panic!("{:?} read its state back under {foreign:?}", S::ALGORITHM);
        };
        assert_eq!(refused.tag(), "no_constructor", "{:?}", S::ALGORITHM);
        assert!(
            refused.detail.contains("a different algorithm"),
            "{:?}: {refused}",
            S::ALGORITHM
        );
    }

    #[test]
    fn every_binding_refuses_a_state_column_carrying_another_algorithms_parameters() {
        refuses_a_foreign_parameter_literal::<KllBinding>(SketchParams::Kll { k: 269 });
        refuses_a_foreign_parameter_literal::<DdSketchBinding>(SketchParams::DDSketch {
            alpha: 0.01,
        });
        refuses_a_foreign_parameter_literal::<HllBinding>(SketchParams::Hll { precision: 14 });
        refuses_a_foreign_parameter_literal::<CountMinBinding>(SketchParams::Cms {
            width: 2048,
            depth: 4,
        });
        refuses_a_foreign_parameter_literal::<CountSketchBinding>(SketchParams::CountSketch {
            width: 2048,
            depth: 4,
        });
        refuses_a_foreign_parameter_literal::<CountMinHeapBinding>(SketchParams::CmsWithHeap {
            width: 2048,
            depth: 4,
            heap_size: 16,
        });
        refuses_a_foreign_parameter_literal::<CountSketchHeapBinding>(
            SketchParams::CountSketchWithHeap {
                width: 2048,
                depth: 4,
                heap_size: 16,
            },
        );
        refuses_a_foreign_parameter_literal::<KmvBinding>(SketchParams::Kmv { k: 1024 });
        refuses_a_foreign_parameter_literal::<UnivMonBinding>(univmon_params());
    }

    #[test]
    fn the_accumulator_reports_the_sketch_footprint_plus_its_own_struct_to_the_ledger() {
        let params = SketchParams::Kll { k: 269 };
        let sketch = KllBinding::bind(&params, SEED).unwrap();
        let footprint = sketch.footprint();
        assert_eq!(footprint, 12_296);

        let ver_one = handle::bind(
            &family(SketchAlgorithm::Kll, params.clone()),
            PostAsapNodeId(1),
            SEED,
        )
        .expect("ver 1 binds the same family");
        assert_eq!(
            ver_one.footprint_bytes(),
            footprint,
            "the two runtimes compute the same footprint"
        );

        let overhead = std::mem::size_of::<SketchAccumulator<KllBinding>>();
        let mut accumulator = SketchAccumulator::<KllBinding>::new(sketch, params);
        assert_eq!(accumulator.size(), overhead + footprint);
        assert_eq!(accumulator.size() - ver_one.footprint_bytes(), overhead);

        for (item, weight) in keyless_rows() {
            accumulator.sketch.update(item.as_ref(), weight).unwrap();
        }
        assert_eq!(
            accumulator.size() - accumulator.sketch.footprint(),
            overhead,
            "the gap is the accumulator struct, once per group, and does not grow with rows"
        );
    }

    #[test]
    fn a_merged_pair_equals_one_matrix_built_from_the_same_rows() {
        let params = SketchParams::Cms {
            width: 2048,
            depth: 4,
        };
        let rows = keyed_rows();
        let mut whole = CountMinBinding::bind(&params, SEED).unwrap();
        for (item, weight) in &rows {
            whole.update(item.as_ref(), *weight).unwrap();
        }
        let (left, right) = rows.split_at(rows.len() / 2);
        let mut first = CountMinBinding::bind(&params, SEED).unwrap();
        for (item, weight) in left {
            first.update(item.as_ref(), *weight).unwrap();
        }
        let mut second = CountMinBinding::bind(&params, SEED).unwrap();
        for (item, weight) in right {
            second.update(item.as_ref(), *weight).unwrap();
        }
        first.merge(&mut second).unwrap();
        assert_eq!(whole.to_bytes().unwrap(), first.to_bytes().unwrap());
    }

    fn three_column_table() -> Arc<MemTable> {
        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("service", ArrowDataType::Utf8, false),
            Field::new("user", ArrowDataType::Utf8, false),
            Field::new("latency", ArrowDataType::Float64, false),
        ]));
        let services: Vec<String> = (0..400)
            .map(|row: i64| if row % 2 == 0 { "a" } else { "b" }.to_owned())
            .collect();
        let users: Vec<String> = (0..400).map(|row: i64| format!("u{}", row % 37)).collect();
        let latencies: Vec<f64> = (0..400)
            .map(|row: i64| ((row * 7919) % 1000) as f64 + 0.5)
            .collect();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(services)),
                Arc::new(StringArray::from(users)),
                Arc::new(Float64Array::from(latencies)),
            ],
        )
        .unwrap();
        Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
    }

    fn rows_of_group(group: &str, keyed: bool) -> Vec<(Option<ItemKey>, f64)> {
        (0..400i64)
            .filter(|row| if row % 2 == 0 { "a" } else { "b" } == group)
            .map(|row| {
                let item = keyed.then(|| ItemKey::Str(format!("u{}", row % 37)));
                let weight = if keyed {
                    1.0
                } else {
                    ((row * 7919) % 1000) as f64 + 0.5
                };
                (item, weight)
            })
            .collect()
    }

    async fn readouts_through_datafusion(
        family: &SummaryFamilyType,
        update: &SummaryUpdate,
        query: &SketchQuery,
    ) -> Vec<(String, Answer)> {
        let session = SeedSession::new(SEED, MemoryPoolSettings::default(), &SummaryFunctions)
            .expect("the session registers the sketch functions");
        let table = three_column_table();
        session
            .context()
            .register_table("t", Arc::clone(&table) as Arc<dyn TableProvider>)
            .unwrap();

        let node = PostAsapNodeId(1);
        let aggregate = sketch_aggregate_call(family, update, node).expect("the family binds");
        let readout = crate::df::estimate_udf::readout_call(family, query, node)
            .expect("the family answers the query");
        let state = session
            .state()
            .aggregate_functions()
            .get(aggregate.function)
            .cloned()
            .expect("the aggregate is registered");
        let estimate = session
            .state()
            .scalar_functions()
            .get(readout.function)
            .cloned()
            .expect("the readout is registered");

        let plan = LogicalPlanBuilder::scan("t", provider_as_source(table), None)
            .unwrap()
            .aggregate(
                vec![col("service")],
                vec![state.call(aggregate.arguments).alias("state")],
            )
            .unwrap()
            .project(vec![
                col("service"),
                estimate
                    .call(readout.arguments(col("state")))
                    .alias("answer"),
            ])
            .unwrap()
            .build()
            .unwrap();

        let physical = session
            .single_mode_physical_plan(&plan)
            .await
            .expect("the physical plan collapses to one aggregate stage");
        let text = displayable(physical.as_ref()).indent(false).to_string();
        assert!(text.contains("mode=Single"), "{text}");
        assert!(!text.contains("mode=Partial"), "{text}");

        let batches = collect(physical, session.context().task_ctx())
            .await
            .expect("the plan runs");
        let mut answers = Vec::new();
        for batch in batches {
            let groups = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let values = batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            for row in 0..batch.num_rows() {
                answers.push((
                    groups.value(row).to_owned(),
                    Answer::Scalar(values.value(row)),
                ));
            }
        }
        answers.sort_by(|left, right| left.0.cmp(&right.0));
        answers
    }

    #[tokio::test]
    async fn a_kll_readout_through_datafusion_equals_the_ver_one_readout() {
        let bound = family(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 });
        let update = SummaryUpdate {
            item: None,
            weight: SummaryInputExpr::Column(ColumnRef::Named("latency".into())),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        let query = SketchQuery::Quantile { q: 0.99 };
        let answers = readouts_through_datafusion(&bound, &update, &query).await;
        assert_eq!(answers.len(), 2);
        for (group, answer) in answers {
            let expected = ver_one_answer(&bound, &rows_of_group(&group, false), &query);
            assert!(
                same_answer(&answer, &expected),
                "group {group}: DataFusion {answer:?} vs ver 1 {expected:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_hll_readout_through_datafusion_equals_the_ver_one_readout() {
        let bound = family(SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 });
        let update = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            weight: SummaryInputExpr::Constant(1.0),
            weight_domain: WeightDomain::NonNegative {
                proof: NonNegativeWeightProof::UnitCount,
            },
        };
        let query = SketchQuery::Cardinality;
        let answers = readouts_through_datafusion(&bound, &update, &query).await;
        assert_eq!(answers.len(), 2);
        for (group, answer) in answers {
            let expected = ver_one_answer(&bound, &rows_of_group(&group, true), &query);
            assert!(
                same_answer(&answer, &expected),
                "group {group}: DataFusion {answer:?} vs ver 1 {expected:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_cms_readout_through_datafusion_equals_the_ver_one_readout() {
        let bound = family(
            SketchAlgorithm::Cms,
            SketchParams::Cms {
                width: 2048,
                depth: 4,
            },
        );
        let update = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            weight: SummaryInputExpr::Constant(1.0),
            weight_domain: WeightDomain::NonNegative {
                proof: NonNegativeWeightProof::UnitCount,
            },
        };
        let query = point_count("u3");
        let answers = readouts_through_datafusion(&bound, &update, &query).await;
        assert_eq!(answers.len(), 2);
        for (group, answer) in answers {
            let expected = ver_one_answer(&bound, &rows_of_group(&group, true), &query);
            assert!(
                same_answer(&answer, &expected),
                "group {group}: DataFusion {answer:?} vs ver 1 {expected:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_ranked_readout_through_datafusion_equals_the_ver_one_ranking() {
        use datafusion::arrow::array::{Int64Array, ListArray, StructArray};

        let bound = family(
            SketchAlgorithm::CmsWithHeap,
            SketchParams::CmsWithHeap {
                width: 2048,
                depth: 4,
                heap_size: 16,
            },
        );
        let update = SummaryUpdate {
            item: Some(SummaryInputExpr::Column(ColumnRef::Named("user".into()))),
            weight: SummaryInputExpr::Constant(1.0),
            weight_domain: WeightDomain::NonNegative {
                proof: NonNegativeWeightProof::UnitCount,
            },
        };
        let query = SketchQuery::TopK { k: 5 };

        let session = SeedSession::new(SEED, MemoryPoolSettings::default(), &SummaryFunctions)
            .expect("the session registers the sketch functions");
        let table = three_column_table();
        session
            .context()
            .register_table("t", Arc::clone(&table) as Arc<dyn TableProvider>)
            .unwrap();

        let node = PostAsapNodeId(1);
        let aggregate = sketch_aggregate_call(&bound, &update, node).unwrap();
        let readout = crate::df::estimate_udf::readout_call(&bound, &query, node).unwrap();
        let state = session
            .state()
            .aggregate_functions()
            .get(aggregate.function)
            .cloned()
            .unwrap();
        let estimate = session
            .state()
            .scalar_functions()
            .get(readout.function)
            .cloned()
            .unwrap();

        let plan = LogicalPlanBuilder::scan("t", provider_as_source(table), None)
            .unwrap()
            .aggregate(
                vec![col("service")],
                vec![state.call(aggregate.arguments).alias("state")],
            )
            .unwrap()
            .project(vec![
                col("service"),
                estimate
                    .call(readout.arguments(col("state")))
                    .alias("answer"),
            ])
            .unwrap()
            .build()
            .unwrap();

        let physical = session.single_mode_physical_plan(&plan).await.unwrap();
        let text = displayable(physical.as_ref()).indent(false).to_string();
        assert!(text.contains("mode=Single"), "{text}");
        assert_eq!(
            physical.schema().field(1).data_type(),
            &crate::df::estimate_udf::ranked_type()
        );

        let batches = collect(physical, session.context().task_ctx())
            .await
            .expect("the plan runs");
        let mut seen = 0usize;
        for batch in batches {
            let groups = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let lists = batch
                .column(1)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();
            for row in 0..batch.num_rows() {
                let entries = lists.value(row);
                let entries = entries.as_any().downcast_ref::<StructArray>().unwrap();
                let keys = entries
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let counts = entries
                    .column(1)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let ranked: Vec<(String, i64)> = (0..entries.len())
                    .map(|entry| (keys.value(entry).to_owned(), counts.value(entry)))
                    .collect();

                let group = groups.value(row);
                let Answer::Ranked(expected) =
                    ver_one_answer(&bound, &rows_of_group(group, true), &query)
                else {
                    panic!("a top-k readout is ranked");
                };
                let expected: Vec<(String, i64)> = expected
                    .into_iter()
                    .map(|(key, count)| (key.rendered(), count as i64))
                    .collect();
                assert_eq!(ranked, expected, "group {group}");
                assert_eq!(ranked.len(), 5);
                seen += 1;
            }
        }
        assert_eq!(seen, 2);
    }

    #[tokio::test]
    async fn the_memory_ledger_charges_the_query_for_the_sketch_state() {
        let bound = family(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 });
        let update = SummaryUpdate {
            item: None,
            weight: SummaryInputExpr::Column(ColumnRef::Named("latency".into())),
            weight_domain: WeightDomain::UnknownOrSigned,
        };
        let settings = MemoryPoolSettings {
            limit_bytes: 8 * 1024,
            ..MemoryPoolSettings::default()
        };
        let session =
            SeedSession::new(SEED, settings, &SummaryFunctions).expect("the session builds");
        let table = three_column_table();
        session
            .context()
            .register_table("t", Arc::clone(&table) as Arc<dyn TableProvider>)
            .unwrap();
        let aggregate = sketch_aggregate_call(&bound, &update, PostAsapNodeId(1)).unwrap();
        let state = session
            .state()
            .aggregate_functions()
            .get(aggregate.function)
            .cloned()
            .unwrap();
        let plan = LogicalPlanBuilder::scan("t", provider_as_source(table), None)
            .unwrap()
            .aggregate(
                vec![col("service")],
                vec![state.call(aggregate.arguments).alias("state")],
            )
            .unwrap()
            .build()
            .unwrap();
        let physical = session.single_mode_physical_plan(&plan).await.unwrap();
        let error = collect(physical, session.context().task_ctx())
            .await
            .expect_err("two 12296-byte sketches do not fit in an 8 KiB pool");
        assert!(
            error.to_string().contains("Resources exhausted"),
            "the pool, not the accumulator, is what refuses: {error}"
        );
    }
}
