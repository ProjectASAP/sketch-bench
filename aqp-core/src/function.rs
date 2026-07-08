//! Executable approximate-function layer for AQPBMV2.
//!
//! This module is intentionally independent of SQL/DataFusion. It gives the
//! benchmark a concrete middle layer between user-visible approximate
//! functionality and raw sketch primitives.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Minimal executable contract for an approximate aggregate function.
///
/// Implementations own their aggregate state. Benchmark kernels own execution
/// shape: single-state, grouped-state, partitioned merge, and later variants.
pub trait ApproxFunction {
    type Input: Clone;
    type State;
    type Output: Clone;
    type Query: Clone;

    fn create(&self) -> Self::State;
    fn update(&self, state: &mut Self::State, input: Self::Input);
    fn merge(&self, left: &mut Self::State, right: Self::State);
    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output;

    fn candidate_id(&self) -> &'static str;
    fn functionality(&self) -> &'static str;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupedInput<T> {
    pub group: String,
    pub value: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelRun<Output> {
    pub kernel: String,
    pub candidate_id: String,
    pub functionality: String,
    pub outputs: BTreeMap<String, Output>,
}

pub fn run_grouped_kernel<F>(
    candidate: &F,
    rows: &[GroupedInput<F::Input>],
    query: F::Query,
) -> KernelRun<F::Output>
where
    F: ApproxFunction,
{
    let mut states: BTreeMap<String, F::State> = BTreeMap::new();
    for row in rows {
        let state = states
            .entry(row.group.clone())
            .or_insert_with(|| candidate.create());
        candidate.update(state, row.value.clone());
    }

    let outputs = states
        .iter_mut()
        .map(|(group, state)| (group.clone(), candidate.finalize(state, query.clone())))
        .collect();

    KernelRun {
        kernel: "grouped_state".to_string(),
        candidate_id: candidate.candidate_id().to_string(),
        functionality: candidate.functionality().to_string(),
        outputs,
    }
}

pub fn run_partitioned_merge_kernel<F>(
    candidate: &F,
    rows: &[GroupedInput<F::Input>],
    partitions: usize,
    query: F::Query,
) -> KernelRun<F::Output>
where
    F: ApproxFunction,
{
    assert!(
        partitions > 0,
        "partitioned kernel needs at least one partition"
    );
    let mut partials: Vec<BTreeMap<String, F::State>> =
        (0..partitions).map(|_| BTreeMap::new()).collect();

    for (idx, row) in rows.iter().enumerate() {
        let partition = idx % partitions;
        let state = partials[partition]
            .entry(row.group.clone())
            .or_insert_with(|| candidate.create());
        candidate.update(state, row.value.clone());
    }

    let mut merged: BTreeMap<String, F::State> = BTreeMap::new();
    for partial in partials {
        for (group, state) in partial {
            match merged.get_mut(&group) {
                Some(left) => candidate.merge(left, state),
                None => {
                    merged.insert(group, state);
                }
            }
        }
    }

    let outputs = merged
        .iter_mut()
        .map(|(group, state)| (group.clone(), candidate.finalize(state, query.clone())))
        .collect();

    KernelRun {
        kernel: format!("partitioned_merge:{partitions}"),
        candidate_id: candidate.candidate_id().to_string(),
        functionality: candidate.functionality().to_string(),
        outputs,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExactCountDistinct;

impl ApproxFunction for ExactCountDistinct {
    type Input = i64;
    type State = HashSet<i64>;
    type Output = f64;
    type Query = ();

    fn create(&self) -> Self::State {
        HashSet::new()
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.insert(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.extend(right);
    }

    fn finalize(&self, state: &mut Self::State, _query: Self::Query) -> Self::Output {
        state.len() as f64
    }

    fn candidate_id(&self) -> &'static str {
        "exact_count_distinct.hashset.v1"
    }

    fn functionality(&self) -> &'static str {
        "count_distinct"
    }
}

#[derive(Debug, Clone)]
pub struct DataSketchesHllCountDistinct {
    lg_k: u8,
    hll_type: datasketches::hll::HllType,
}

impl DataSketchesHllCountDistinct {
    pub fn new(lg_k: u8) -> Self {
        Self {
            lg_k,
            hll_type: datasketches::hll::HllType::Hll8,
        }
    }
}

impl ApproxFunction for DataSketchesHllCountDistinct {
    type Input = i64;
    type State = datasketches::hll::HllSketch;
    type Output = f64;
    type Query = ();

    fn create(&self) -> Self::State {
        datasketches::hll::HllSketch::new(self.lg_k, self.hll_type)
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        let mut union = datasketches::hll::HllUnion::new(self.lg_k);
        union.update(left);
        union.update(&right);
        *left = union.get_result(self.hll_type);
    }

    fn finalize(&self, state: &mut Self::State, _query: Self::Query) -> Self::Output {
        state.estimate()
    }

    fn candidate_id(&self) -> &'static str {
        "approx_count_distinct.datasketches_hll.v1"
    }

    fn functionality(&self) -> &'static str {
        "count_distinct"
    }
}

#[derive(Debug, Clone)]
pub struct SketchOxideHllCountDistinct {
    precision: u8,
}

impl SketchOxideHllCountDistinct {
    pub fn new(precision: u8) -> Self {
        Self { precision }
    }
}

impl ApproxFunction for SketchOxideHllCountDistinct {
    type Input = i64;
    type State = sketch_oxide::cardinality::HyperLogLog;
    type Output = f64;
    type Query = ();

    fn create(&self) -> Self::State {
        sketch_oxide::cardinality::HyperLogLog::new(self.precision)
            .expect("valid sketch_oxide HLL precision")
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(&input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        sketch_oxide::Mergeable::merge(left, &right).expect("compatible sketch_oxide HLL states");
    }

    fn finalize(&self, state: &mut Self::State, _query: Self::Query) -> Self::Output {
        sketch_oxide::Sketch::estimate(state)
    }

    fn candidate_id(&self) -> &'static str {
        "approx_count_distinct.sketch_oxide_hll.v1"
    }

    fn functionality(&self) -> &'static str {
        "count_distinct"
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AsapHllCountDistinct;

impl ApproxFunction for AsapHllCountDistinct {
    type Input = i64;
    type State = asap_sketchlib::HyperLogLog<asap_sketchlib::Classic>;
    type Output = f64;
    type Query = ();

    fn create(&self) -> Self::State {
        asap_sketchlib::HyperLogLog::<asap_sketchlib::Classic>::new()
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.insert(&asap_sketchlib::DataInput::I64(input));
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right);
    }

    fn finalize(&self, state: &mut Self::State, _query: Self::Query) -> Self::Output {
        state.estimate() as f64
    }

    fn candidate_id(&self) -> &'static str {
        "approx_count_distinct.asap_sketchlib_hll_classic.v1"
    }

    fn functionality(&self) -> &'static str {
        "count_distinct"
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TopKQuery {
    pub k: usize,
    pub min_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavyHitter {
    pub item: i64,
    pub estimate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavyHitters {
    pub items: Vec<HeavyHitter>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExactHeavyHitters;

impl ApproxFunction for ExactHeavyHitters {
    type Input = i64;
    type State = HashMap<i64, u64>;
    type Output = HeavyHitters;
    type Query = TopKQuery;

    fn create(&self) -> Self::State {
        HashMap::new()
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        *state.entry(input).or_insert(0) += 1;
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        for (item, count) in right {
            *left.entry(item).or_insert(0) += count;
        }
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        let mut items: Vec<_> = state
            .iter()
            .filter(|(_, count)| **count >= query.min_count)
            .map(|(item, count)| HeavyHitter {
                item: *item,
                estimate: *count as f64,
            })
            .collect();
        sort_and_truncate_heavy_hitters(&mut items, query.k);
        HeavyHitters { items }
    }

    fn candidate_id(&self) -> &'static str {
        "exact_heavy_hitters.hashmap.v1"
    }

    fn functionality(&self) -> &'static str {
        "heavy_hitters"
    }
}

#[derive(Debug, Clone)]
pub struct DataSketchesFrequentItemsHeavyHitters {
    max_map_size: usize,
}

impl DataSketchesFrequentItemsHeavyHitters {
    pub fn new(max_map_size: usize) -> Self {
        Self { max_map_size }
    }
}

impl ApproxFunction for DataSketchesFrequentItemsHeavyHitters {
    type Input = i64;
    type State = datasketches::frequencies::FrequentItemsSketch<i64>;
    type Output = HeavyHitters;
    type Query = TopKQuery;

    fn create(&self) -> Self::State {
        datasketches::frequencies::FrequentItemsSketch::new(self.max_map_size)
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right);
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        let mut items: Vec<_> = state
            .frequent_items_with_threshold(
                datasketches::frequencies::ErrorType::NoFalseNegatives,
                query.min_count,
            )
            .into_iter()
            .map(|row| HeavyHitter {
                item: *row.item(),
                estimate: row.estimate() as f64,
            })
            .collect();
        sort_and_truncate_heavy_hitters(&mut items, query.k);
        HeavyHitters { items }
    }

    fn candidate_id(&self) -> &'static str {
        "approx_heavy_hitters.datasketches_frequent_items.v1"
    }

    fn functionality(&self) -> &'static str {
        "heavy_hitters"
    }
}

#[derive(Debug, Clone)]
pub struct SketchOxideSpaceSavingHeavyHitters {
    capacity: usize,
}

impl SketchOxideSpaceSavingHeavyHitters {
    pub fn new(capacity: usize) -> Self {
        Self { capacity }
    }
}

impl ApproxFunction for SketchOxideSpaceSavingHeavyHitters {
    type Input = i64;
    type State = sketch_oxide::frequency::SpaceSaving<i64>;
    type Output = HeavyHitters;
    type Query = TopKQuery;

    fn create(&self) -> Self::State {
        sketch_oxide::frequency::SpaceSaving::with_capacity(self.capacity)
            .expect("valid sketch_oxide SpaceSaving capacity")
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right)
            .expect("compatible sketch_oxide SpaceSaving states");
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        let mut items: Vec<_> = state
            .top_k(query.k)
            .into_iter()
            .filter(|(_, _, upper)| *upper >= query.min_count)
            .map(|(item, _lower, upper)| HeavyHitter {
                item,
                estimate: upper as f64,
            })
            .collect();
        sort_and_truncate_heavy_hitters(&mut items, query.k);
        HeavyHitters { items }
    }

    fn candidate_id(&self) -> &'static str {
        "approx_heavy_hitters.sketch_oxide_space_saving.v1"
    }

    fn functionality(&self) -> &'static str {
        "heavy_hitters"
    }
}

#[derive(Debug, Clone)]
pub struct AsapCmsHeapHeavyHitters {
    rows: usize,
    cols: usize,
    top_k: usize,
}

impl AsapCmsHeapHeavyHitters {
    pub fn new(rows: usize, cols: usize, top_k: usize) -> Self {
        Self { rows, cols, top_k }
    }
}

impl ApproxFunction for AsapCmsHeapHeavyHitters {
    type Input = i64;
    type State = asap_sketchlib::CMSHeap<asap_sketchlib::Vector2D<i64>, asap_sketchlib::FastPath>;
    type Output = HeavyHitters;
    type Query = TopKQuery;

    fn create(&self) -> Self::State {
        asap_sketchlib::CMSHeap::<asap_sketchlib::Vector2D<i64>, asap_sketchlib::FastPath>::new(
            self.rows, self.cols, self.top_k,
        )
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.insert(&asap_sketchlib::DataInput::I64(input));
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right);
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        let mut items: Vec<_> = state
            .heap()
            .heap()
            .iter()
            .filter(|item| item.count >= query.min_count as i64)
            .filter_map(|item| {
                heap_item_to_i64(&item.key).map(|key| HeavyHitter {
                    item: key,
                    estimate: item.count as f64,
                })
            })
            .collect();
        sort_and_truncate_heavy_hitters(&mut items, query.k);
        HeavyHitters { items }
    }

    fn candidate_id(&self) -> &'static str {
        "approx_heavy_hitters.asap_sketchlib_cms_heap.v1"
    }

    fn functionality(&self) -> &'static str {
        "heavy_hitters"
    }
}

fn heap_item_to_i64(item: &asap_sketchlib::HeapItem) -> Option<i64> {
    match item {
        asap_sketchlib::HeapItem::I8(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::I16(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::I32(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::I64(v) => Some(*v),
        asap_sketchlib::HeapItem::ISIZE(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::U8(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::U16(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::U32(v) => Some(*v as i64),
        asap_sketchlib::HeapItem::U64(v) => i64::try_from(*v).ok(),
        asap_sketchlib::HeapItem::USIZE(v) => i64::try_from(*v).ok(),
        _ => None,
    }
}

fn sort_and_truncate_heavy_hitters(items: &mut Vec<HeavyHitter>, k: usize) {
    items.sort_by(|a, b| {
        b.estimate
            .partial_cmp(&a.estimate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.item.cmp(&b.item))
    });
    items.truncate(k);
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QuantileQuery {
    pub quantile: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExactQuantile;

impl ApproxFunction for ExactQuantile {
    type Input = f64;
    type State = Vec<f64>;
    type Output = f64;
    type Query = QuantileQuery;

    fn create(&self) -> Self::State {
        Vec::new()
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        if input.is_finite() {
            state.push(input);
        }
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.extend(right);
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        nearest_rank_quantile(state, query.quantile)
    }

    fn candidate_id(&self) -> &'static str {
        "exact_quantile.sorted_vec.v1"
    }

    fn functionality(&self) -> &'static str {
        "quantile"
    }
}

#[derive(Debug, Clone)]
pub struct AsapKllQuantile {
    k: i32,
    seed: u64,
}

impl AsapKllQuantile {
    pub fn new(k: i32) -> Self {
        Self {
            k,
            seed: 0x5155_434b_4c4c,
        }
    }
}

impl ApproxFunction for AsapKllQuantile {
    type Input = f64;
    type State = asap_sketchlib::KLL<f64>;
    type Output = f64;
    type Query = QuantileQuery;

    fn create(&self) -> Self::State {
        asap_sketchlib::KLL::<f64>::init_kll_with_seed(self.k, self.seed)
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        if input.is_finite() {
            state.update(&input);
        }
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right);
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        state.quantile(query.quantile)
    }

    fn candidate_id(&self) -> &'static str {
        "approx_quantile.asap_sketchlib_kll.v1"
    }

    fn functionality(&self) -> &'static str {
        "quantile"
    }
}

#[derive(Debug, Clone)]
pub struct DataSketchesTDigestQuantile {
    k: u16,
}

impl DataSketchesTDigestQuantile {
    pub fn new(k: u16) -> Self {
        Self { k }
    }
}

impl ApproxFunction for DataSketchesTDigestQuantile {
    type Input = f64;
    type State = datasketches::tdigest::TDigestMut;
    type Output = f64;
    type Query = QuantileQuery;

    fn create(&self) -> Self::State {
        datasketches::tdigest::TDigestMut::new(self.k)
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        left.merge(&right);
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        state.quantile(query.quantile).unwrap_or(0.0)
    }

    fn candidate_id(&self) -> &'static str {
        "approx_quantile.datasketches_tdigest.v1"
    }

    fn functionality(&self) -> &'static str {
        "quantile"
    }
}

#[derive(Debug, Clone)]
pub struct SketchOxideTDigestQuantile {
    compression: f64,
}

impl SketchOxideTDigestQuantile {
    pub fn new(compression: f64) -> Self {
        Self { compression }
    }
}

impl ApproxFunction for SketchOxideTDigestQuantile {
    type Input = f64;
    type State = sketch_oxide::quantiles::TDigest;
    type Output = f64;
    type Query = QuantileQuery;

    fn create(&self) -> Self::State {
        sketch_oxide::quantiles::TDigest::new(self.compression)
    }

    fn update(&self, state: &mut Self::State, input: Self::Input) {
        state.update(input);
    }

    fn merge(&self, left: &mut Self::State, right: Self::State) {
        sketch_oxide::Mergeable::merge(left, &right)
            .expect("compatible sketch_oxide TDigest states");
    }

    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output {
        state.quantile(query.quantile)
    }

    fn candidate_id(&self) -> &'static str {
        "approx_quantile.sketch_oxide_tdigest.v1"
    }

    fn functionality(&self) -> &'static str {
        "quantile"
    }
}

fn nearest_rank_quantile(values: &mut [f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = quantile.clamp(0.0, 1.0);
    let idx = if q == 0.0 {
        0
    } else {
        ((values.len() as f64 * q).ceil() as usize).saturating_sub(1)
    };
    values[idx.min(values.len() - 1)]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumericCoverageSummary {
    pub compared_groups: usize,
    pub groups_within_relative_error: usize,
    pub answer_coverage: f64,
    pub mean_relative_error: f64,
    pub max_relative_error: f64,
    pub worst_group: Option<GroupComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupComparison {
    pub group: String,
    pub exact: f64,
    pub approximate: f64,
    pub relative_error: f64,
}

pub fn compare_numeric_outputs(
    exact: &BTreeMap<String, f64>,
    approximate: &BTreeMap<String, f64>,
    relative_error_threshold: f64,
) -> NumericCoverageSummary {
    let mut comparisons = Vec::new();
    for (group, exact_value) in exact {
        if let Some(approximate_value) = approximate.get(group) {
            let relative_error = if *exact_value == 0.0 {
                if *approximate_value == 0.0 {
                    0.0
                } else {
                    1.0
                }
            } else {
                (approximate_value - exact_value).abs() / exact_value.abs()
            };
            comparisons.push(GroupComparison {
                group: group.clone(),
                exact: *exact_value,
                approximate: *approximate_value,
                relative_error,
            });
        }
    }

    let compared_groups = comparisons.len();
    let groups_within_relative_error = comparisons
        .iter()
        .filter(|c| c.relative_error <= relative_error_threshold)
        .count();
    let mean_relative_error = if compared_groups == 0 {
        0.0
    } else {
        comparisons.iter().map(|c| c.relative_error).sum::<f64>() / compared_groups as f64
    };
    let max_relative_error = comparisons
        .iter()
        .map(|c| c.relative_error)
        .fold(0.0, f64::max);
    let worst_group = comparisons.into_iter().max_by(|a, b| {
        a.relative_error
            .partial_cmp(&b.relative_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    NumericCoverageSummary {
        compared_groups,
        groups_within_relative_error,
        answer_coverage: if compared_groups == 0 {
            0.0
        } else {
            groups_within_relative_error as f64 / compared_groups as f64
        },
        mean_relative_error,
        max_relative_error,
        worst_group,
    }
}

pub fn compare_count_distinct_outputs(
    exact: &BTreeMap<String, f64>,
    approximate: &BTreeMap<String, f64>,
    relative_error_threshold: f64,
) -> NumericCoverageSummary {
    compare_numeric_outputs(exact, approximate, relative_error_threshold)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavyHitterCoverageSummary {
    pub compared_groups: usize,
    pub mean_precision_at_k: f64,
    pub mean_recall_at_k: f64,
    pub groups_with_full_recall: usize,
}

pub fn compare_heavy_hitter_outputs(
    exact: &BTreeMap<String, HeavyHitters>,
    approximate: &BTreeMap<String, HeavyHitters>,
) -> HeavyHitterCoverageSummary {
    let mut compared_groups = 0;
    let mut precision_sum = 0.0;
    let mut recall_sum = 0.0;
    let mut groups_with_full_recall = 0;

    for (group, exact_value) in exact {
        if let Some(approximate_value) = approximate.get(group) {
            compared_groups += 1;
            let exact_items: HashSet<_> = exact_value.items.iter().map(|item| item.item).collect();
            let approximate_items: HashSet<_> = approximate_value
                .items
                .iter()
                .map(|item| item.item)
                .collect();
            let intersection = exact_items.intersection(&approximate_items).count();
            let precision = if approximate_items.is_empty() {
                if exact_items.is_empty() {
                    1.0
                } else {
                    0.0
                }
            } else {
                intersection as f64 / approximate_items.len() as f64
            };
            let recall = if exact_items.is_empty() {
                1.0
            } else {
                intersection as f64 / exact_items.len() as f64
            };
            precision_sum += precision;
            recall_sum += recall;
            if (recall - 1.0).abs() <= f64::EPSILON {
                groups_with_full_recall += 1;
            }
        }
    }

    HeavyHitterCoverageSummary {
        compared_groups,
        mean_precision_at_k: if compared_groups == 0 {
            0.0
        } else {
            precision_sum / compared_groups as f64
        },
        mean_recall_at_k: if compared_groups == 0 {
            0.0
        } else {
            recall_sum / compared_groups as f64
        },
        groups_with_full_recall,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountDistinctCandidateReport {
    pub grouped: KernelRun<f64>,
    pub partitioned_merge: KernelRun<f64>,
    pub grouped_coverage: NumericCoverageSummary,
    pub partitioned_coverage: NumericCoverageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavyHitterCandidateReport {
    pub grouped: KernelRun<HeavyHitters>,
    pub partitioned_merge: KernelRun<HeavyHitters>,
    pub grouped_coverage: HeavyHitterCoverageSummary,
    pub partitioned_coverage: HeavyHitterCoverageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantileCandidateReport {
    pub grouped: KernelRun<f64>,
    pub partitioned_merge: KernelRun<f64>,
    pub grouped_coverage: NumericCoverageSummary,
    pub partitioned_coverage: NumericCoverageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aqpbmv2FunctionDemoReport {
    pub count_distinct_exact: KernelRun<f64>,
    pub count_distinct_candidates: Vec<CountDistinctCandidateReport>,
    pub heavy_hitter_exact: KernelRun<HeavyHitters>,
    pub heavy_hitter_candidates: Vec<HeavyHitterCandidateReport>,
    pub quantile_exact: KernelRun<f64>,
    pub quantile_candidates: Vec<QuantileCandidateReport>,
}

pub fn demo_count_distinct_rows(
    row_count: usize,
    group_count: usize,
    value_cardinality: i64,
) -> Vec<GroupedInput<i64>> {
    assert!(group_count > 0, "demo needs at least one group");
    assert!(
        value_cardinality > 0,
        "demo needs positive value cardinality"
    );
    (0..row_count)
        .map(|idx| {
            let group = format!("group_{:03}", idx % group_count);
            let value = ((idx as i64 * 37) + ((idx / group_count) as i64 * 17))
                .rem_euclid(value_cardinality);
            GroupedInput { group, value }
        })
        .collect()
}

pub fn demo_heavy_hitter_rows(row_count: usize, group_count: usize) -> Vec<GroupedInput<i64>> {
    assert!(group_count > 0, "demo needs at least one group");
    (0..row_count)
        .map(|idx| {
            let group_idx = idx % group_count;
            let sequence = idx / group_count;
            let bucket = sequence % 100;
            let base = group_idx as i64 * 10_000;
            let value = match bucket {
                0..=44 => base,
                45..=64 => base + 1,
                65..=79 => base + 2,
                _ => base + 1_000 + (sequence % 251) as i64,
            };
            GroupedInput {
                group: format!("group_{group_idx:03}"),
                value,
            }
        })
        .collect()
}

pub fn demo_quantile_rows(row_count: usize, group_count: usize) -> Vec<GroupedInput<f64>> {
    assert!(group_count > 0, "demo needs at least one group");
    (0..row_count)
        .map(|idx| {
            let group_idx = idx % group_count;
            let sequence = (idx / group_count) as f64;
            let periodic = (sequence * 13.0).rem_euclid(100.0);
            let tail = if (sequence as u64).is_multiple_of(20) {
                (sequence * 0.37).rem_euclid(700.0)
            } else {
                0.0
            };
            GroupedInput {
                group: format!("group_{group_idx:03}"),
                value: group_idx as f64 * 10.0 + periodic + tail,
            }
        })
        .collect()
}

pub fn run_aqpbmv2_function_demo() -> Aqpbmv2FunctionDemoReport {
    let count_rows = demo_count_distinct_rows(50_000, 32, 20_000);
    let count_exact = ExactCountDistinct;
    let count_distinct_exact = run_grouped_kernel(&count_exact, &count_rows, ());
    let count_distinct_candidates = vec![
        evaluate_count_distinct_candidate(
            &DataSketchesHllCountDistinct::new(12),
            &count_rows,
            &count_distinct_exact,
        ),
        evaluate_count_distinct_candidate(
            &SketchOxideHllCountDistinct::new(12),
            &count_rows,
            &count_distinct_exact,
        ),
        evaluate_count_distinct_candidate(
            &AsapHllCountDistinct,
            &count_rows,
            &count_distinct_exact,
        ),
    ];

    let heavy_rows = demo_heavy_hitter_rows(80_000, 24);
    let heavy_query = TopKQuery { k: 3, min_count: 1 };
    let heavy_exact = ExactHeavyHitters;
    let heavy_hitter_exact = run_grouped_kernel(&heavy_exact, &heavy_rows, heavy_query);
    let heavy_hitter_candidates = vec![
        evaluate_heavy_hitter_candidate(
            &DataSketchesFrequentItemsHeavyHitters::new(256),
            &heavy_rows,
            heavy_query,
            &heavy_hitter_exact,
        ),
        evaluate_heavy_hitter_candidate(
            &SketchOxideSpaceSavingHeavyHitters::new(128),
            &heavy_rows,
            heavy_query,
            &heavy_hitter_exact,
        ),
        evaluate_heavy_hitter_candidate(
            &AsapCmsHeapHeavyHitters::new(5, 2048, heavy_query.k),
            &heavy_rows,
            heavy_query,
            &heavy_hitter_exact,
        ),
    ];

    let quantile_rows = demo_quantile_rows(80_000, 16);
    let quantile_query = QuantileQuery { quantile: 0.95 };
    let quantile_exact_function = ExactQuantile;
    let quantile_exact =
        run_grouped_kernel(&quantile_exact_function, &quantile_rows, quantile_query);
    let quantile_candidates = vec![
        evaluate_quantile_candidate(
            &DataSketchesTDigestQuantile::new(200),
            &quantile_rows,
            quantile_query,
            &quantile_exact,
        ),
        evaluate_quantile_candidate(
            &SketchOxideTDigestQuantile::new(200.0),
            &quantile_rows,
            quantile_query,
            &quantile_exact,
        ),
        evaluate_quantile_candidate(
            &AsapKllQuantile::new(200),
            &quantile_rows,
            quantile_query,
            &quantile_exact,
        ),
    ];

    Aqpbmv2FunctionDemoReport {
        count_distinct_exact,
        count_distinct_candidates,
        heavy_hitter_exact,
        heavy_hitter_candidates,
        quantile_exact,
        quantile_candidates,
    }
}

fn evaluate_count_distinct_candidate<F>(
    candidate: &F,
    rows: &[GroupedInput<i64>],
    exact: &KernelRun<f64>,
) -> CountDistinctCandidateReport
where
    F: ApproxFunction<Input = i64, Output = f64, Query = ()>,
{
    let grouped = run_grouped_kernel(candidate, rows, ());
    let partitioned_merge = run_partitioned_merge_kernel(candidate, rows, 8, ());
    let grouped_coverage = compare_numeric_outputs(&exact.outputs, &grouped.outputs, 0.05);
    let partitioned_coverage =
        compare_numeric_outputs(&exact.outputs, &partitioned_merge.outputs, 0.05);
    CountDistinctCandidateReport {
        grouped,
        partitioned_merge,
        grouped_coverage,
        partitioned_coverage,
    }
}

fn evaluate_heavy_hitter_candidate<F>(
    candidate: &F,
    rows: &[GroupedInput<i64>],
    query: TopKQuery,
    exact: &KernelRun<HeavyHitters>,
) -> HeavyHitterCandidateReport
where
    F: ApproxFunction<Input = i64, Output = HeavyHitters, Query = TopKQuery>,
{
    let grouped = run_grouped_kernel(candidate, rows, query);
    let partitioned_merge = run_partitioned_merge_kernel(candidate, rows, 8, query);
    let grouped_coverage = compare_heavy_hitter_outputs(&exact.outputs, &grouped.outputs);
    let partitioned_coverage =
        compare_heavy_hitter_outputs(&exact.outputs, &partitioned_merge.outputs);
    HeavyHitterCandidateReport {
        grouped,
        partitioned_merge,
        grouped_coverage,
        partitioned_coverage,
    }
}

fn evaluate_quantile_candidate<F>(
    candidate: &F,
    rows: &[GroupedInput<f64>],
    query: QuantileQuery,
    exact: &KernelRun<f64>,
) -> QuantileCandidateReport
where
    F: ApproxFunction<Input = f64, Output = f64, Query = QuantileQuery>,
{
    let grouped = run_grouped_kernel(candidate, rows, query);
    let partitioned_merge = run_partitioned_merge_kernel(candidate, rows, 8, query);
    let grouped_coverage = compare_numeric_outputs(&exact.outputs, &grouped.outputs, 0.08);
    let partitioned_coverage =
        compare_numeric_outputs(&exact.outputs, &partitioned_merge.outputs, 0.08);
    QuantileCandidateReport {
        grouped,
        partitioned_merge,
        grouped_coverage,
        partitioned_coverage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_grouped_count_distinct_is_correct() {
        let rows = vec![
            GroupedInput {
                group: "a".to_string(),
                value: 1,
            },
            GroupedInput {
                group: "a".to_string(),
                value: 1,
            },
            GroupedInput {
                group: "a".to_string(),
                value: 2,
            },
            GroupedInput {
                group: "b".to_string(),
                value: 9,
            },
        ];
        let run = run_grouped_kernel(&ExactCountDistinct, &rows, ());
        assert_eq!(run.outputs["a"], 2.0);
        assert_eq!(run.outputs["b"], 1.0);
    }

    #[test]
    fn partitioned_exact_merge_matches_grouped_exact() {
        let rows = demo_count_distinct_rows(10_000, 17, 3_000);
        let grouped = run_grouped_kernel(&ExactCountDistinct, &rows, ());
        let partitioned = run_partitioned_merge_kernel(&ExactCountDistinct, &rows, 7, ());
        assert_eq!(grouped.outputs, partitioned.outputs);
    }

    #[test]
    fn real_hll_candidates_produce_count_distinct_coverage() {
        let rows = demo_count_distinct_rows(20_000, 16, 8_000);
        let exact = run_grouped_kernel(&ExactCountDistinct, &rows, ());
        for report in [
            evaluate_count_distinct_candidate(
                &DataSketchesHllCountDistinct::new(12),
                &rows,
                &exact,
            ),
            evaluate_count_distinct_candidate(&SketchOxideHllCountDistinct::new(12), &rows, &exact),
            evaluate_count_distinct_candidate(&AsapHllCountDistinct, &rows, &exact),
        ] {
            assert_eq!(report.grouped_coverage.compared_groups, 16);
            assert_eq!(report.partitioned_coverage.compared_groups, 16);
            assert!(report.grouped_coverage.answer_coverage >= 0.80);
            assert!(report.partitioned_coverage.answer_coverage >= 0.80);
        }
    }

    #[test]
    fn heavy_hitter_candidates_recover_top_items() {
        let rows = demo_heavy_hitter_rows(40_000, 10);
        let query = TopKQuery { k: 3, min_count: 1 };
        let exact = run_grouped_kernel(&ExactHeavyHitters, &rows, query);
        for coverage in [
            evaluate_heavy_hitter_candidate(
                &DataSketchesFrequentItemsHeavyHitters::new(256),
                &rows,
                query,
                &exact,
            )
            .grouped_coverage,
            evaluate_heavy_hitter_candidate(
                &SketchOxideSpaceSavingHeavyHitters::new(128),
                &rows,
                query,
                &exact,
            )
            .grouped_coverage,
            evaluate_heavy_hitter_candidate(
                &AsapCmsHeapHeavyHitters::new(5, 2048, query.k),
                &rows,
                query,
                &exact,
            )
            .grouped_coverage,
        ] {
            assert_eq!(coverage.compared_groups, 10);
            assert!(coverage.mean_recall_at_k >= 0.90);
        }
    }

    #[test]
    fn quantile_candidates_produce_numeric_coverage() {
        let rows = demo_quantile_rows(40_000, 10);
        let query = QuantileQuery { quantile: 0.95 };
        let exact = run_grouped_kernel(&ExactQuantile, &rows, query);
        for coverage in [
            evaluate_quantile_candidate(
                &DataSketchesTDigestQuantile::new(200),
                &rows,
                query,
                &exact,
            )
            .grouped_coverage,
            evaluate_quantile_candidate(
                &SketchOxideTDigestQuantile::new(200.0),
                &rows,
                query,
                &exact,
            )
            .grouped_coverage,
            evaluate_quantile_candidate(&AsapKllQuantile::new(200), &rows, query, &exact)
                .grouped_coverage,
        ] {
            assert_eq!(coverage.compared_groups, 10);
            assert!(coverage.answer_coverage >= 0.80);
        }
    }
}
