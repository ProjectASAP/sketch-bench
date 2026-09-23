//! SBT-1 — bind a planner-chosen summary family to a concrete constructor, or
//! refuse it by name.
//!
//! v0 binds the nine algorithms `asap_sketchlib` implements — `Kll`,
//! `DDSketch`, `Hll`, `Cms`, `CountSketch`, `CmsWithHeap`,
//! `CountSketchWithHeap`, `Kmv` and `UnivMon` — plus the four order-independent
//! exact accumulators. `Theta`, the order-dependent exact accumulators and
//! every other family are refused by name, because a substitution (answering
//! `Theta` with HLL, say) would leave the readout's `ResultGuarantee`
//! describing an algorithm that did not run, voiding the accuracy claim while
//! every number still looked reasonable.

use asap_sketchlib::common::heap::HHHeap;
use asap_sketchlib::common::input::HHItem;
use asap_sketchlib::input::{DataInput, HeapItem};
use asap_sketchlib::sketch_framework::univmon::UnivMon;
use asap_sketchlib::sketches::hll::HyperLogLogImpl;
use asap_sketchlib::{
    CMSHeap, CSHeap, Classic, Count, CountMin, DDSketch, FastPath, HllBucketListP12,
    HllBucketListP14, HllBucketListP16, HllRegisterStorage, Vector2D, KLL, KMV,
};
use asap_types::post_asap::{
    ExactKind, ExactParams, GroupingStrategy, PostAsapNodeId, SketchAlgorithm, SketchCategory,
    SketchParams, SketchQuery, SummaryFamilyType,
};
use asap_types::pre_asap::ColumnRef;

use crate::types::{Answer, EvalError, ItemKey, Refusal};

/// `asap_sketchlib` silently clamps `k` into this range inside
/// `KLL::init_internal`: `norm_k = k.max(norm_m)` raises anything below the
/// floor and `if norm_k > MAX_CACHEABLE_K { norm_k = MAX_CACHEABLE_K }`
/// truncates anything above the ceiling (`src/sketches/kll.rs:29,303-309`).
/// Both are silent, so a `k` outside this range would leave the recorded `k`
/// different from the `k` that ran.
pub const KLL_K_MIN: u32 = 8;
/// See [`KLL_K_MIN`]. This is `MAX_CACHEABLE_K` (`kll.rs:29`).
pub const KLL_K_MAX: u32 = 26_602;

/// `MAX_LEVELS` (`kll.rs:26`) — the compactor-level count `compute_max_capacity`
/// sums over.
pub const KLL_MAX_LEVELS: usize = 61;
/// `CAPACITY_DECAY` (`kll.rs:30`).
const KLL_CAPACITY_DECAY: f64 = 2.0 / 3.0;
/// The `m` that `init_kll_with_seed` passes to `init` (`kll.rs:291-298`).
pub const KLL_M: usize = 8;

/// Replica of the private `compute_max_capacity` (`kll.rs:119-127`). A replica
/// rather than a `k * 4` approximation because the reported footprint is the
/// denominator of this crate's headline ratio.
pub fn kll_max_capacity(k: usize, m: usize) -> usize {
    let mut total = 0usize;
    let mut scale = 1.0_f64;
    for _ in 0..KLL_MAX_LEVELS {
        total += ((k as f64) * scale).ceil().max(m as f64) as usize;
        scale *= KLL_CAPACITY_DECAY;
    }
    total
}

/// One live summary instance, one per `(node, group)` the run holds state for.
pub trait SummaryHandle {
    /// Feed one observation. `item` is `Some` only for keyed families; every
    /// family bound in v0 is keyless and refuses a `Some` rather than quietly
    /// reading whichever half it understands.
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError>;
    /// Read a value out. `&mut self` because `KLL::quantile_cached` is
    /// `&mut self` (`kll.rs:580`) — a `&self` signature cannot drive it.
    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError>;
    /// The family this handle was bound from, as the plan named it.
    fn family(&self) -> &SummaryFamilyType;
    /// Bytes of summary state held, excluding the handle's own bookkeeping.
    fn footprint_bytes(&self) -> usize;
}

/// Whether `family` can be bound, without constructing anything.
///
/// Admission calls this so an unbindable family is refused before the first row
/// is read rather than at row zero of the first group.
pub fn check_bindable(family: &SummaryFamilyType, node: PostAsapNodeId) -> Result<(), Refusal> {
    match family {
        SummaryFamilyType::Sketch(kind, grouping) => {
            // Grouping first: a Hydra layout changes what the params mean, so
            // the algorithm arm would be answering the wrong question.
            match grouping {
                GroupingStrategy::PerSubpopulationInstance => {}
                GroupingStrategy::SharedMultiSubpopulation { kind, .. } => {
                    return Err(Refusal::UnsupportedGrouping {
                        node,
                        detail: format!(
                            "SharedMultiSubpopulation({kind:?}): the IR gives no mapping from \
                             `by` keys to the shared layout"
                        ),
                    })
                }
            }
            // Re-check what `SketchKind::new`'s assertion would have caught.
            // Deserialization bypasses it (private fields + derived
            // `Deserialize`), so a document declaring an impossible triple
            // round-trips and validates green.
            match (kind.category(), kind.algorithm(), kind.params()) {
                (category, algorithm, params)
                    if !params_match_algorithm(algorithm, params)
                        || !category_matches_algorithm(&category, algorithm) =>
                {
                    return Err(Refusal::InconsistentSketchKind {
                        node,
                        detail: format!("{category:?} / {algorithm:?} / {params:?}"),
                    })
                }
                _ => {}
            }
            let unbound = |reason: &str| {
                Err(Refusal::UnboundFamily {
                    node,
                    family: Box::new(SummaryFamilyType::Sketch(kind.clone(), grouping.clone())),
                    reason: reason.to_owned(),
                })
            };
            let out_of_bounds =
                |detail: String| Err(Refusal::ParameterOutOfBounds { node, detail });
            // Exhaustive on purpose: `SketchAlgorithm` is not
            // `#[non_exhaustive]`, so an eleventh algorithm upstream fails this
            // build instead of landing in a wildcard and being refused at run
            // time on some later corpus sweep.
            match (kind.algorithm(), kind.params()) {
                (SketchAlgorithm::Kll, SketchParams::Kll { k }) => {
                    if !(KLL_K_MIN..=KLL_K_MAX).contains(k) {
                        return out_of_bounds(format!(
                            "Kll k = {k} is outside [{KLL_K_MIN}, {KLL_K_MAX}]; the library would \
                             clamp it silently and the recorded k would not be the k that ran"
                        ));
                    }
                    Ok(())
                }
                (SketchAlgorithm::DDSketch, SketchParams::DDSketch { alpha }) => {
                    if !(*alpha > 0.0 && *alpha < 1.0) {
                        return out_of_bounds(format!(
                            "DDSketch alpha = {alpha} is outside (0, 1)"
                        ));
                    }
                    Ok(())
                }
                (SketchAlgorithm::Hll, SketchParams::Hll { precision }) => match precision {
                    12 | 14 | 16 => Ok(()),
                    other => out_of_bounds(format!(
                        "Hll precision = {other}: asap_sketchlib carries precision in the \
                             register-storage type and implements only 12, 14 and 16; rounding \
                             would make the recorded precision differ from the one that ran"
                    )),
                },
                (SketchAlgorithm::Cms, SketchParams::Cms { width, depth })
                | (SketchAlgorithm::CountSketch, SketchParams::CountSketch { width, depth }) => {
                    check_matrix_shape(node, *width, *depth)
                }
                (
                    SketchAlgorithm::CmsWithHeap,
                    SketchParams::CmsWithHeap {
                        width,
                        depth,
                        heap_size,
                    },
                )
                | (
                    SketchAlgorithm::CountSketchWithHeap,
                    SketchParams::CountSketchWithHeap {
                        width,
                        depth,
                        heap_size,
                    },
                ) => {
                    check_matrix_shape(node, *width, *depth)?;
                    if *heap_size == 0 {
                        return out_of_bounds("heap_size = 0 keeps no heavy hitter".to_owned());
                    }
                    Ok(())
                }
                (SketchAlgorithm::Kmv, SketchParams::Kmv { k }) => {
                    if *k == 0 {
                        return out_of_bounds("Kmv k = 0 retains no minimum".to_owned());
                    }
                    Ok(())
                }
                (
                    SketchAlgorithm::UnivMon,
                    SketchParams::UnivMon {
                        heap_size,
                        sketch_rows,
                        sketch_cols,
                        layers,
                    },
                ) => {
                    check_matrix_shape(node, *sketch_cols, *sketch_rows)?;
                    if *heap_size == 0 || *layers == 0 {
                        return out_of_bounds(format!(
                            "UnivMon heap_size = {heap_size}, layers = {layers}: neither may be 0"
                        ));
                    }
                    Ok(())
                }
                (SketchAlgorithm::Theta, _) => unbound(
                    "asap_sketchlib has no Theta sketch at all, and answering it with HLL would \
                     leave the readout's ResultGuarantee describing an algorithm that did not run",
                ),
                (SketchAlgorithm::Kll, params)
                | (SketchAlgorithm::DDSketch, params)
                | (SketchAlgorithm::Hll, params)
                | (SketchAlgorithm::Cms, params)
                | (SketchAlgorithm::CountSketch, params)
                | (SketchAlgorithm::CmsWithHeap, params)
                | (SketchAlgorithm::CountSketchWithHeap, params)
                | (SketchAlgorithm::Kmv, params)
                | (SketchAlgorithm::UnivMon, params) => Err(Refusal::InconsistentSketchKind {
                    node,
                    detail: format!("{:?} / {params:?}", kind.algorithm()),
                }),
            }
        }
        SummaryFamilyType::ExactAggregate(kind, params) => match (kind, params) {
            (ExactKind::Sum, ExactParams::Sum)
            | (ExactKind::Count, ExactParams::Count)
            | (ExactKind::Min, ExactParams::Min)
            | (ExactKind::Max, ExactParams::Max) => Ok(()),
            // Increase/Rate/IRate hold state that depends on sample order and
            // window duration, and neither reaches `update(item, weight)`.
            (ExactKind::Increase, ExactParams::Increase)
            | (ExactKind::Rate, ExactParams::Rate)
            | (ExactKind::IRate, ExactParams::IRate) => Err(Refusal::UnboundFamily {
                node,
                family: Box::new(family.clone()),
                reason: "state depends on sample order and window duration, neither of which \
                         reaches `update(item, weight)`"
                    .to_owned(),
            }),
            (kind, params) => Err(Refusal::InconsistentSketchKind {
                node,
                detail: format!("ExactAggregate({kind:?}, {params:?})"),
            }),
        },
        other => Err(Refusal::UnboundFamily {
            node,
            family: Box::new(other.clone()),
            reason: "v0 binds only Sketch and ExactAggregate families".to_owned(),
        }),
    }
}

pub fn answers_the_same_question(family: &SummaryFamilyType, query: &SketchQuery) -> bool {
    match family {
        SummaryFamilyType::Sketch(kind, _) => matches!(
            (kind.algorithm(), query),
            (
                SketchAlgorithm::Kll | SketchAlgorithm::DDSketch,
                SketchQuery::Quantile { .. }
            ) | (
                SketchAlgorithm::Hll | SketchAlgorithm::Kmv | SketchAlgorithm::Theta,
                SketchQuery::Cardinality
            ) | (
                SketchAlgorithm::Cms
                    | SketchAlgorithm::CountSketch
                    | SketchAlgorithm::CmsWithHeap
                    | SketchAlgorithm::CountSketchWithHeap,
                SketchQuery::PointCount { value: Some(_), .. }
            ) | (
                // The bare bucket total. Every CMS row receives every insert
                // exactly once, so one row sums to the total weight ingested
                // — the same number the exact arm gets from summing the
                // weights. A Count-Min row carries no signs, which is why the
                // other three algorithms above are not in this arm: a
                // CountSketch row sums over ±1 hashes and a heap-backed
                // variant answers membership, not a total.
                SketchAlgorithm::Cms,
                SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                }
            ) | (
                SketchAlgorithm::CmsWithHeap | SketchAlgorithm::CountSketchWithHeap,
                SketchQuery::TopK { .. }
            ) | (
                SketchAlgorithm::UnivMon,
                SketchQuery::Cardinality | SketchQuery::FrequencyL2 | SketchQuery::FrequencyEntropy
            )
        ),
        SummaryFamilyType::ExactAggregate(kind, _) => match (kind, query) {
            (
                ExactKind::Sum,
                SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                },
            )
            | (
                ExactKind::Count,
                SketchQuery::PointCount {
                    key: ColumnRef::Wildcard,
                    value: None,
                },
            ) => true,
            (ExactKind::Min, SketchQuery::Quantile { q }) => *q <= 0.0,
            (ExactKind::Max, SketchQuery::Quantile { q }) => *q >= 1.0,
            _ => false,
        },
        _ => false,
    }
}

pub fn check_readout(
    family: &SummaryFamilyType,
    query: &SketchQuery,
    node: PostAsapNodeId,
) -> Result<(), Refusal> {
    if answers_the_same_question(family, query) {
        return Ok(());
    }
    Err(Refusal::FamilyDoesNotAnswerReadout {
        node,
        family: Box::new(family.clone()),
        query: Box::new(query.clone()),
    })
}

fn category_matches_algorithm(category: &SketchCategory, algorithm: &SketchAlgorithm) -> bool {
    matches!(
        (category, algorithm),
        (SketchCategory::Quantile, SketchAlgorithm::Kll)
            | (SketchCategory::Quantile, SketchAlgorithm::DDSketch)
            | (SketchCategory::Cardinality, SketchAlgorithm::Hll)
            | (SketchCategory::Cardinality, SketchAlgorithm::Kmv)
            | (SketchCategory::Cardinality, SketchAlgorithm::Theta)
            | (SketchCategory::Frequency, SketchAlgorithm::Cms)
            | (SketchCategory::Frequency, SketchAlgorithm::CountSketch)
            | (SketchCategory::TopK, SketchAlgorithm::CmsWithHeap)
            | (SketchCategory::TopK, SketchAlgorithm::CountSketchWithHeap)
            | (SketchCategory::Universal, SketchAlgorithm::UnivMon)
    )
}

fn params_match_algorithm(algorithm: &SketchAlgorithm, params: &SketchParams) -> bool {
    matches!(
        (algorithm, params),
        (SketchAlgorithm::Kll, SketchParams::Kll { .. })
            | (SketchAlgorithm::DDSketch, SketchParams::DDSketch { .. })
            | (SketchAlgorithm::Hll, SketchParams::Hll { .. })
            | (SketchAlgorithm::Kmv, SketchParams::Kmv { .. })
            | (SketchAlgorithm::Theta, SketchParams::Theta { .. })
            | (SketchAlgorithm::Cms, SketchParams::Cms { .. })
            | (
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch { .. }
            )
            | (
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap { .. }
            )
            | (
                SketchAlgorithm::CountSketchWithHeap,
                SketchParams::CountSketchWithHeap { .. }
            )
            | (SketchAlgorithm::UnivMon, SketchParams::UnivMon { .. })
    )
}

/// Construct the handle. `check_bindable` must have passed.
pub fn bind(
    family: &SummaryFamilyType,
    node: PostAsapNodeId,
    seed: u64,
) -> Result<Box<dyn SummaryHandle>, Refusal> {
    check_bindable(family, node)?;
    match family {
        SummaryFamilyType::Sketch(kind, _) => match kind.params() {
            SketchParams::Kll { k } => Ok(Box::new(KllHandle::new(family.clone(), node, *k, seed))),
            SketchParams::DDSketch { alpha } => Ok(Box::new(DdHandle {
                family: family.clone(),
                node,
                inner: DDSketch::new(*alpha),
            })),
            SketchParams::Hll { precision } => match precision {
                12 => Ok(Box::new(HllHandle::<HllBucketListP12> {
                    family: family.clone(),
                    node,
                    inner: HyperLogLogImpl::<Classic, HllBucketListP12>::new(),
                })),
                14 => Ok(Box::new(HllHandle::<HllBucketListP14> {
                    family: family.clone(),
                    node,
                    inner: HyperLogLogImpl::<Classic, HllBucketListP14>::new(),
                })),
                _ => Ok(Box::new(HllHandle::<HllBucketListP16> {
                    family: family.clone(),
                    node,
                    inner: HyperLogLogImpl::<Classic, HllBucketListP16>::new(),
                })),
            },
            SketchParams::Cms { width, depth } => {
                let (rows, cols) = transposed(*width, *depth);
                Ok(Box::new(CmsHandle {
                    family: family.clone(),
                    node,
                    inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(rows, cols),
                    rows,
                    cols,
                    ingested: 0,
                }))
            }
            SketchParams::CountSketch { width, depth } => {
                let (rows, cols) = transposed(*width, *depth);
                Ok(Box::new(CsHandle {
                    family: family.clone(),
                    node,
                    inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(rows, cols),
                    rows,
                    cols,
                    ingested: 0,
                }))
            }
            SketchParams::CmsWithHeap {
                width,
                depth,
                heap_size,
            } => {
                let (rows, cols) = transposed(*width, *depth);
                let heap_size = *heap_size as usize;
                Ok(Box::new(CmsHeapHandle {
                    family: family.clone(),
                    node,
                    inner: CMSHeap::<Vector2D<i32>, FastPath>::new(rows, cols, heap_size),
                    rows,
                    cols,
                    heap_size,
                    ingested: 0,
                }))
            }
            SketchParams::CountSketchWithHeap {
                width,
                depth,
                heap_size,
            } => {
                let (rows, cols) = transposed(*width, *depth);
                let heap_size = *heap_size as usize;
                Ok(Box::new(CsHeapHandle {
                    family: family.clone(),
                    node,
                    inner: CSHeap::<Vector2D<i32>, FastPath>::new(rows, cols, heap_size),
                    rows,
                    cols,
                    heap_size,
                    ingested: 0,
                }))
            }
            SketchParams::Kmv { k } => {
                let k = *k as usize;
                Ok(Box::new(KmvHandle {
                    family: family.clone(),
                    node,
                    inner: KMV::new(k),
                    k,
                }))
            }
            SketchParams::UnivMon {
                heap_size,
                sketch_rows,
                sketch_cols,
                layers,
            } => {
                let (heap_size, rows, cols, layers) = (
                    *heap_size as usize,
                    *sketch_rows as usize,
                    *sketch_cols as usize,
                    *layers as usize,
                );
                Ok(Box::new(UnivMonHandle {
                    family: family.clone(),
                    node,
                    inner: UnivMon::init_univmon(heap_size, rows, cols, layers),
                    heap_size,
                    rows,
                    cols,
                    layers,
                }))
            }
            SketchParams::Theta { .. } => {
                unreachable!("check_bindable refuses Theta: asap_sketchlib has no implementation")
            }
        },
        SummaryFamilyType::ExactAggregate(kind, _) => {
            Ok(Box::new(ExactHandle::new(family.clone(), node, kind)))
        }
        _ => unreachable!("check_bindable admits only Sketch and ExactAggregate"),
    }
}

/// IR `width` is the library's `cols` and IR `depth` is its `rows`.
pub fn transposed(width: u32, depth: u32) -> (usize, usize) {
    (depth as usize, width as usize)
}

fn check_matrix_shape(node: PostAsapNodeId, width: u32, depth: u32) -> Result<(), Refusal> {
    if width == 0 || depth == 0 {
        return Err(Refusal::ParameterOutOfBounds {
            node,
            detail: format!(
                "width = {width}, depth = {depth}: `Vector2D::init` takes `cols.ilog2()`, which \
                 aborts at 0, and a zero-row matrix answers every query out of an empty fold"
            ),
        });
    }
    Ok(())
}

fn require_item<'a>(
    node: PostAsapNodeId,
    item: Option<&'a ItemKey>,
    family: &str,
) -> Result<DataInput<'a>, EvalError> {
    match item {
        Some(ItemKey::Str(s)) => Ok(DataInput::Str(s)),
        Some(ItemKey::Int(i)) => Ok(DataInput::I64(*i)),
        Some(ItemKey::Float(f)) => Ok(DataInput::F64(*f)),
        None => Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
            node,
            detail: format!("{family} is keyed; `input.item` must name a column"),
        }])),
    }
}

fn require_unit_weight(node: PostAsapNodeId, weight: f64, family: &str) -> Result<(), EvalError> {
    if weight == 1.0 {
        return Ok(());
    }
    Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
        node,
        detail: format!(
            "{family} has no weighted insert in asap_sketchlib; weight {weight} would be dropped"
        ),
    }]))
}

fn charge_i32_total(
    node: PostAsapNodeId,
    ingested: &mut i64,
    many: i32,
    weight: f64,
    family: &str,
) -> Result<(), EvalError> {
    let next = *ingested + i64::from(many);
    if next > i64::from(i32::MAX) {
        return Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
            node,
            detail: format!(
                "{family} counts in i32; {} already ingested plus weight {weight} would exceed \
                 i32::MAX = {}, and a single hot key can land the whole total in one counter",
                *ingested,
                i32::MAX
            ),
        }]));
    }
    *ingested = next;
    Ok(())
}

fn require_i32_weight(node: PostAsapNodeId, weight: f64, family: &str) -> Result<i32, EvalError> {
    if weight.is_finite() && weight.fract() == 0.0 && weight >= 0.0 && weight <= f64::from(i32::MAX)
    {
        return Ok(weight as i32);
    }
    Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
        node,
        detail: format!(
            "{family} counts in i32; weight {weight} is not a non-negative integer it can hold"
        ),
    }]))
}

fn wrong_query(node: PostAsapNodeId, query: &SketchQuery) -> EvalError {
    EvalError::Refused(vec![Refusal::UnsupportedReadout {
        node,
        query: Box::new(query.clone()),
    }])
}

fn point_key<'a>(node: PostAsapNodeId, query: &'a SketchQuery) -> Result<DataInput<'a>, EvalError> {
    match query {
        SketchQuery::PointCount {
            value: Some(value), ..
        } => Ok(DataInput::Str(value)),
        other => Err(wrong_query(node, other)),
    }
}

fn reject_item(
    node: PostAsapNodeId,
    item: Option<&ItemKey>,
    family: &str,
) -> Result<(), EvalError> {
    match item {
        None => Ok(()),
        Some(_) => Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
            node,
            detail: format!("{family} is keyless; `input.item` must be absent"),
        }])),
    }
}

struct DdHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: DDSketch,
}

impl SummaryHandle for DdHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        reject_item(self.node, item, "DDSketch")?;
        if !weight.is_finite() || weight <= 0.0 {
            return Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
                node: self.node,
                detail: format!(
                    "DDSketch::add drops non-positive and non-finite values with no error \
                     channel; {weight} would leave the summary describing fewer rows than were \
                     scanned"
                ),
            }]));
        }
        self.inner.add(&weight);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match query {
            SketchQuery::Quantile { q } => Ok(Answer::Scalar(
                self.inner.get_value_at_quantile(*q).unwrap_or(f64::NAN),
            )),
            other => Err(wrong_query(self.node, other)),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        std::mem::size_of_val(self.inner.store_counts())
    }
}

struct HllHandle<R: HllRegisterStorage> {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: HyperLogLogImpl<Classic, R>,
}

impl<R: HllRegisterStorage> SummaryHandle for HllHandle<R> {
    fn update(&mut self, item: Option<&ItemKey>, _weight: f64) -> Result<(), EvalError> {
        let key = require_item(self.node, item, "Hll")?;
        self.inner.insert(&key);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(self.inner.estimate() as f64)),
            other => Err(wrong_query(self.node, other)),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        R::NUM_REGISTERS
    }
}

struct CmsHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: CountMin<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    ingested: i64,
}

impl CmsHandle {
    /// The total weight this sketch ingested, read off row 0.
    ///
    /// `Vector2D::fast_insert` adds `many` to exactly one column of every row
    /// (`vector2d.rs:283-294`), so a row's sum is the total, with no collision
    /// loss: two keys sharing a column add up in that column rather than
    /// overwriting each other. `check_matrix_shape` has already refused a
    /// zero-row matrix, so row 0 exists.
    fn ingested_weight(&self) -> f64 {
        let counts = self.inner.as_storage();
        (0..self.cols)
            .map(|col| f64::from(counts.query_one_counter(0, col)))
            .sum()
    }
}

impl SummaryHandle for CmsHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        let many = require_i32_weight(self.node, weight, "Cms")?;
        let key = require_item(self.node, item, "Cms")?;
        charge_i32_total(self.node, &mut self.ingested, many, weight, "Cms")?;
        self.inner.insert_many(&key, many);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        if let SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        } = query
        {
            return Ok(Answer::Scalar(self.ingested_weight()));
        }
        let key = point_key(self.node, query)?;
        Ok(Answer::Scalar(f64::from(self.inner.estimate(&key))))
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}

struct CsHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: Count<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    ingested: i64,
}

impl SummaryHandle for CsHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        let many = require_i32_weight(self.node, weight, "CountSketch")?;
        let key = require_item(self.node, item, "CountSketch")?;
        charge_i32_total(self.node, &mut self.ingested, many, weight, "CountSketch")?;
        self.inner.insert_many(&key, many);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        let key = point_key(self.node, query)?;
        Ok(Answer::Scalar(self.inner.estimate(&key)))
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}

struct CmsHeapHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: CMSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    heap_size: usize,
    ingested: i64,
}

impl SummaryHandle for CmsHeapHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        require_unit_weight(self.node, weight, "CmsWithHeap")?;
        let key = require_item(self.node, item, "CmsWithHeap")?;
        charge_i32_total(self.node, &mut self.ingested, 1, weight, "CmsWithHeap")?;
        self.inner.insert(&key);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        if let SketchQuery::TopK { k } = query {
            return heap_topk(self.node, self.inner.heap(), *k);
        }
        let key = point_key(self.node, query)?;
        Ok(Answer::Scalar(f64::from(self.inner.estimate(&key))))
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
            + heap_bytes(self.inner.heap(), self.heap_size)
    }
}

struct CsHeapHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: CSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    heap_size: usize,
    ingested: i64,
}

impl SummaryHandle for CsHeapHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        require_unit_weight(self.node, weight, "CountSketchWithHeap")?;
        let key = require_item(self.node, item, "CountSketchWithHeap")?;
        charge_i32_total(
            self.node,
            &mut self.ingested,
            1,
            weight,
            "CountSketchWithHeap",
        )?;
        self.inner.insert(&key);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        if let SketchQuery::TopK { k } = query {
            return heap_topk(self.node, self.inner.heap(), *k);
        }
        let key = point_key(self.node, query)?;
        Ok(Answer::Scalar(self.inner.estimate(&key)))
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
            + heap_bytes(self.inner.heap(), self.heap_size)
    }
}

pub const SKETCHLIB_PREALLOCATED_SLOTS: usize = 1024;

const SKETCHLIB_HEAP_INDEX_ENTRY_BYTES: usize = 8 + 24 + 1;

pub fn grown_capacity(reserved: usize, len: usize) -> usize {
    let mut capacity = reserved.max(1);
    while capacity < len {
        capacity *= 2;
    }
    if reserved == 0 && len == 0 {
        return 0;
    }
    capacity
}

fn hash_buckets(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let wanted = capacity.div_ceil(7) * 8;
    wanted.next_power_of_two()
}

pub fn heap_bytes(heap: &HHHeap, heap_size: usize) -> usize {
    let residents = heap.heap();
    let capacity = grown_capacity(heap_size.min(SKETCHLIB_PREALLOCATED_SLOTS), residents.len());
    let keys: usize = residents
        .iter()
        .map(|item| match &item.key {
            HeapItem::String(held) => held.capacity(),
            HeapItem::Bytes(held) => held.capacity(),
            _ => 0,
        })
        .sum();
    capacity * std::mem::size_of::<HHItem>()
        + capacity * std::mem::size_of::<u64>()
        + hash_buckets(capacity) * SKETCHLIB_HEAP_INDEX_ENTRY_BYTES
        + keys
}

fn heap_topk(node: PostAsapNodeId, heap: &HHHeap, k: usize) -> Result<Answer, EvalError> {
    let mut ranked: Vec<(ItemKey, u64)> = Vec::with_capacity(heap.len());
    for item in heap.heap() {
        ranked.push((heap_key(node, &item.key)?, item.count.max(0) as u64));
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

fn heap_key(node: PostAsapNodeId, item: &HeapItem) -> Result<ItemKey, EvalError> {
    match item {
        HeapItem::String(held) => Ok(ItemKey::Str(held.clone())),
        HeapItem::I64(held) => Ok(ItemKey::Int(*held)),
        HeapItem::F64(held) => Ok(ItemKey::Float(*held)),
        other => Err(EvalError::Handle(format!(
            "node {node:?}: the heap holds a {other:?} key, which no update this crate \
             performs could have put there"
        ))),
    }
}

struct KmvHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: KMV,
    k: usize,
}

impl SummaryHandle for KmvHandle {
    fn update(&mut self, item: Option<&ItemKey>, _weight: f64) -> Result<(), EvalError> {
        let key = require_item(self.node, item, "Kmv")?;
        self.inner.insert(&key);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(self.inner.estimate())),
            other => Err(wrong_query(self.node, other)),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        let capacity = grown_capacity(
            self.k.min(SKETCHLIB_PREALLOCATED_SLOTS),
            self.inner.k_vals.len(),
        );
        capacity * std::mem::size_of::<u64>()
    }
}

struct UnivMonHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: UnivMon,
    heap_size: usize,
    rows: usize,
    cols: usize,
    layers: usize,
}

impl SummaryHandle for UnivMonHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        let count = require_i32_weight(self.node, weight, "UnivMon")?;
        let key = require_item(self.node, item, "UnivMon")?;
        self.inner.insert(&key, i64::from(count));
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match query {
            SketchQuery::Cardinality => Ok(Answer::Scalar(self.inner.calc_card())),
            SketchQuery::FrequencyL2 => Ok(Answer::Scalar(self.inner.calc_l2())),
            SketchQuery::FrequencyEntropy => Ok(Answer::Scalar(self.inner.calc_entropy())),
            other => Err(wrong_query(self.node, other)),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        let counters = (self.rows * self.cols + self.rows) * std::mem::size_of::<i64>();
        let heaps: usize = self
            .inner
            .hh_layers
            .iter()
            .map(|heap| heap_bytes(heap, self.heap_size))
            .sum();
        self.layers * counters + heaps + self.layers
    }
}

// ── KLL ──────────────────────────────────────────────────────────────────────

/// `Kll{k}` over `f64`. `KLL<T>` orders `T` by `total_cmp` and converts
/// nothing, so the item type is the library's own choice.
struct KllHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    inner: KLL<f64>,
    k: u32,
}

impl KllHandle {
    fn new(family: SummaryFamilyType, node: PostAsapNodeId, k: u32, seed: u64) -> Self {
        Self {
            family,
            node,
            // Seeded, not `init_kll`: the same seed and the same input sequence
            // give a byte-identical state, which is what makes a multi-seed
            // sweep reproducible one seed at a time.
            inner: KLL::<f64>::init_kll_with_seed(k as i32, seed),
            k,
        }
    }
}

impl SummaryHandle for KllHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        // `KLL::update(&mut self, val: &T)` takes no weight at all
        // (`kll.rs:408`). The weight *is* the value being summarized.
        reject_item(self.node, item, "Kll")?;
        self.inner.update(&weight);
        Ok(())
    }

    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match query {
            SketchQuery::Quantile { q } => Ok(Answer::Scalar(self.inner.quantile_cached(*q))),
            other => Err(EvalError::Refused(vec![Refusal::UnsupportedReadout {
                node: self.node,
                query: Box::new(other.clone()),
            }])),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        // Three allocations, all made up front by `init_internal`
        // (`kll.rs:304-326`): the item buffer, the level array, and the merge
        // buffer. At k=269 that is 9648 + 496 + 2152 = 12296 B; omitting the
        // merge buffer understates it by 17.5%, and this number is the
        // denominator of the memory advantage ratio.
        let items = kll_max_capacity(self.k as usize, KLL_M) * std::mem::size_of::<f64>();
        let levels = (KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
        let merge_buf = self.k as usize * std::mem::size_of::<f64>();
        items + levels + merge_buf
    }
}

// ── Exact accumulators ───────────────────────────────────────────────────────

/// Sum / Count / Min / Max. No library involved, and the only handles for which an
/// inverse operation could ever be implemented honestly.
struct ExactHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    state: ExactState,
}

/// One accumulator, because one exact kind has one answer. Holding all four
/// and reading one back would make `footprint_bytes` report a quarter of the
/// state the handle really carries, and the reported number is the numerator
/// of this crate's headline ratio.
enum ExactState {
    Sum(f64),
    Count(u64),
    Min(f64),
    Max(f64),
}

impl ExactHandle {
    fn new(family: SummaryFamilyType, node: PostAsapNodeId, kind: &ExactKind) -> Self {
        let state = match kind {
            ExactKind::Sum => ExactState::Sum(0.0),
            ExactKind::Count => ExactState::Count(0),
            ExactKind::Min => ExactState::Min(f64::INFINITY),
            ExactKind::Max => ExactState::Max(f64::NEG_INFINITY),
            other => unreachable!(
                "check_bindable refuses the order-dependent exact accumulators: {other:?}"
            ),
        };
        Self {
            family,
            node,
            state,
        }
    }
}

impl SummaryHandle for ExactHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        reject_item(self.node, item, "an exact accumulator")?;
        match &mut self.state {
            ExactState::Sum(sum) => *sum += weight,
            ExactState::Count(count) => *count += 1,
            ExactState::Min(min) => *min = min.min(weight),
            ExactState::Max(max) => *max = max.max(weight),
        }
        Ok(())
    }

    /// An exact accumulator's state *is* its value, so the corpus reaches it
    /// through `FinalizeExactAccumulator` rather than a `SummaryEstimate`. This
    /// arm exists for a caller that asks anyway; the mapping is our convention,
    /// not something the IR specifies, and it is exactly the one
    /// [`answers_the_same_question`] admits.
    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        if !answers_the_same_question(&self.family, query) {
            return Err(EvalError::Refused(vec![Refusal::UnsupportedReadout {
                node: self.node,
                query: Box::new(query.clone()),
            }]));
        }
        match (&self.state, query) {
            (ExactState::Sum(sum), SketchQuery::PointCount { .. }) => Ok(Answer::Scalar(*sum)),
            (ExactState::Count(count), SketchQuery::PointCount { .. }) => {
                Ok(Answer::Scalar(*count as f64))
            }
            // `Answer::Scalar` holds one number, so each of Min and Max
            // answers only its own endpoint of the quantile range.
            (ExactState::Min(min), SketchQuery::Quantile { .. }) => Ok(Answer::Scalar(*min)),
            (ExactState::Max(max), SketchQuery::Quantile { .. }) => Ok(Answer::Scalar(*max)),
            (_, other) => Err(EvalError::Refused(vec![Refusal::UnsupportedReadout {
                node: self.node,
                query: Box::new(other.clone()),
            }])),
        }
    }

    fn family(&self) -> &SummaryFamilyType {
        &self.family
    }

    fn footprint_bytes(&self) -> usize {
        std::mem::size_of::<ExactState>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_types::post_asap::SketchKind;

    fn kll(k: u32) -> SummaryFamilyType {
        SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k }),
            GroupingStrategy::PerSubpopulationInstance,
        )
    }

    fn exact(kind: ExactKind, params: ExactParams) -> SummaryFamilyType {
        SummaryFamilyType::ExactAggregate(kind, params)
    }

    const NODE: PostAsapNodeId = PostAsapNodeId(1);

    #[test]
    fn every_algorithm_asap_sketchlib_implements_binds() {
        let cases = [
            (
                SketchAlgorithm::DDSketch,
                SketchParams::DDSketch { alpha: 0.01 },
            ),
            (SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 }),
            (
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
            (
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch {
                    width: 30_000,
                    depth: 5,
                },
            ),
            (
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap {
                    width: 272,
                    depth: 5,
                    heap_size: 64,
                },
            ),
            (
                SketchAlgorithm::CountSketchWithHeap,
                SketchParams::CountSketchWithHeap {
                    width: 272,
                    depth: 5,
                    heap_size: 64,
                },
            ),
            (SketchAlgorithm::Kmv, SketchParams::Kmv { k: 1_000_002 }),
            (
                SketchAlgorithm::UnivMon,
                SketchParams::UnivMon {
                    heap_size: 1000,
                    sketch_rows: 5,
                    sketch_cols: 272,
                    layers: 16,
                },
            ),
        ];

        assert_eq!(cases.len(), 8, "nine algorithms, Kll plus these eight bind");
        for (algorithm, params) in cases {
            let family = SummaryFamilyType::Sketch(
                SketchKind::new(algorithm.clone(), params),
                GroupingStrategy::PerSubpopulationInstance,
            );
            check_bindable(&family, NODE)
                .unwrap_or_else(|e| panic!("{algorithm:?} must be bindable, got {e:?}"));
            let handle = bind(&family, NODE, 7)
                .unwrap_or_else(|e| panic!("{algorithm:?} must construct, got {e:?}"));
            assert!(
                handle.footprint_bytes() > 0 || matches!(algorithm, SketchAlgorithm::DDSketch),
                "{algorithm:?} must report a footprint before it holds anything"
            );
        }
    }

    #[test]
    fn a_heap_bearing_sketch_is_charged_for_the_whole_heap() {
        let heap_size = 64;
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap {
                    width: 272,
                    depth: 5,
                    heap_size,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );

        let counters = 5 * 272 * std::mem::size_of::<i32>();
        let mut handle = bind(&family, NODE, 0).expect("binds");
        let empty = handle.footprint_bytes();
        assert!(
            empty > counters,
            "the heap is reserved at construction and is not free"
        );

        for i in 0..10_000u64 {
            handle
                .update(Some(&ItemKey::Str(format!("series-{}", i % 500))), 1.0)
                .unwrap();
        }
        let held = handle.footprint_bytes() - counters;

        let items = heap_size as usize * std::mem::size_of::<HHItem>();
        let digests = heap_size as usize * std::mem::size_of::<u64>();
        let index = hash_buckets(heap_size as usize) * SKETCHLIB_HEAP_INDEX_ENTRY_BYTES;
        let keys: usize = (0..heap_size as usize).map(|_| "series-000".len()).sum();
        assert!(
            held >= items + digests + index,
            "the array, the digest column and the position index are all held: {held}"
        );
        assert!(
            held >= items + digests + index + keys / 2,
            "the residents' own key bytes are held too: {held}"
        );

        let guessed = heap_size as usize * 24;
        assert!(
            held as f64 / guessed as f64 > 3.0,
            "24 B per slot counts the array short and the index not at all, \
             {held} against {guessed}"
        );
    }

    #[test]
    fn a_kmv_is_charged_for_what_it_allocated_and_not_for_the_k_it_was_asked_for() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Kmv, SketchParams::Kmv { k: 1_000_002 }),
            GroupingStrategy::PerSubpopulationInstance,
        );

        let mut handle = bind(&family, NODE, 0).expect("binds");
        for i in 0..10_000u64 {
            handle.update(Some(&ItemKey::Int(i as i64)), 1.0).unwrap();
        }

        let held = handle.footprint_bytes();
        assert_eq!(
            held,
            16_384 * std::mem::size_of::<u64>(),
            "1024 slots reserved, doubled to hold 10k hashes"
        );
        assert!(
            held < 1_000_002 * std::mem::size_of::<u64>() / 8,
            "the requested k is 8 MB and nothing like it was allocated"
        );
    }

    #[test]
    fn an_hll_precision_the_library_cannot_build_is_refused_rather_than_rounded() {
        for precision in [11, 13, 15, 17] {
            let family = SummaryFamilyType::Sketch(
                SketchKind::new(SketchAlgorithm::Hll, SketchParams::Hll { precision }),
                GroupingStrategy::PerSubpopulationInstance,
            );
            assert!(
                matches!(
                    check_bindable(&family, NODE),
                    Err(Refusal::ParameterOutOfBounds { .. })
                ),
                "precision {precision} must be refused, not rounded to a neighbour"
            );
        }
    }

    #[test]
    fn a_keyed_family_refuses_a_row_with_no_item_rather_than_reading_the_weight() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let mut handle = bind(&family, NODE, 0).expect("binds");
        assert!(handle.update(None, 1.0).is_err(), "Cms is keyed");
        assert!(handle.update(Some(&ItemKey::Str("a".into())), 3.0).is_ok());
    }

    #[test]
    fn ddsketch_refuses_the_values_the_library_would_drop_in_silence() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::DDSketch,
                SketchParams::DDSketch { alpha: 0.01 },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let mut handle = bind(&family, NODE, 0).expect("binds");
        for dropped in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                handle.update(None, dropped).is_err(),
                "{dropped} is dropped by `add` with no error channel"
            );
        }
        assert!(handle.update(None, 12.0).is_ok());
    }

    #[test]
    fn the_matrix_is_transposed_between_the_ir_and_the_library() {
        // IR width -> library cols, IR depth -> library rows. A square shape
        // would not catch a swap, so the two differ.
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 256,
                    depth: 4,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let handle = bind(&family, NODE, 0).expect("binds");
        assert_eq!(
            handle.footprint_bytes(),
            4 * 256 * std::mem::size_of::<i32>()
        );
    }

    #[test]
    fn kll_answers_a_quantile_within_the_planners_claimed_rank_error() {
        let mut handle = bind(&kll(269), NODE, 42).expect("k = 269 binds");
        for i in 0..10_000 {
            handle.update(None, i as f64).unwrap();
        }
        let answer = handle.estimate(&SketchQuery::Quantile { q: 0.5 }).unwrap();
        let estimate = match answer {
            Answer::Scalar(v) => v,
            other => panic!("expected a scalar, got {other:?}"),
        };
        // The planner claims 2.296 / 269^0.9723 = 0.009966 rank error.
        let rank_error = (estimate / 10_000.0 - 0.5).abs();
        assert!(rank_error <= 0.009966, "rank error {rank_error}");
    }

    #[test]
    fn a_k_the_library_would_silently_clamp_is_refused_rather_than_run() {
        // The library raises anything below 8 and truncates anything above
        // MAX_CACHEABLE_K, both silently, so the recorded k would not be the
        // k that ran. Verified against the library itself below.
        for k in [0, 7, KLL_K_MAX + 1, 65_535] {
            assert!(
                matches!(
                    check_bindable(&kll(k), NODE),
                    Err(Refusal::ParameterOutOfBounds { .. })
                ),
                "k = {k} must be refused"
            );
        }
        assert!(check_bindable(&kll(KLL_K_MIN), NODE).is_ok());
        assert!(check_bindable(&kll(KLL_K_MAX), NODE).is_ok());
    }

    #[test]
    fn the_library_really_does_clamp_k_silently() {
        // Pins the reason the bound above exists. If this ever fails, the
        // clamp changed and KLL_K_MIN/KLL_K_MAX must be re-derived.
        assert_eq!(KLL::<f64>::init_kll_with_seed(30_000, 1).k(), 26_602);
        assert_eq!(KLL::<f64>::init_kll_with_seed(7, 1).k(), 8);
    }

    #[test]
    fn theta_is_refused_by_name_and_never_substituted() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(SketchAlgorithm::Theta, SketchParams::Theta { k: 4096 }),
            GroupingStrategy::PerSubpopulationInstance,
        );
        match check_bindable(&family, NODE) {
            Err(Refusal::UnboundFamily { reason, .. }) => {
                assert!(reason.contains("no Theta sketch"), "got {reason:?}");
                assert!(
                    reason.contains("HLL"),
                    "must say why substituting is worse: {reason:?}"
                );
            }
            other => panic!("Theta must stay refused, got {other:?}"),
        }
    }

    #[test]
    fn an_inconsistent_sketch_kind_is_caught_even_though_serde_let_it_in() {
        // `SketchKind::new` asserts this triple is impossible, but private
        // fields plus a derived `Deserialize` mean a document can carry it.
        let kind: SketchKind = serde_json::from_str(
            r#"{"category":"Cardinality","algorithm":"Kll","params":{"Kll":{"k":269}}}"#,
        )
        .expect("serde accepts what the constructor would not");
        let family = SummaryFamilyType::Sketch(kind, GroupingStrategy::PerSubpopulationInstance);
        assert!(matches!(
            check_bindable(&family, NODE),
            Err(Refusal::InconsistentSketchKind { .. })
        ));
    }

    #[test]
    fn exact_accumulators_are_exact() {
        let mut sum = bind(&exact(ExactKind::Sum, ExactParams::Sum), NODE, 0).unwrap();
        let mut min = bind(&exact(ExactKind::Min, ExactParams::Min), NODE, 0).unwrap();
        let mut max = bind(&exact(ExactKind::Max, ExactParams::Max), NODE, 0).unwrap();
        for i in 1..=100 {
            sum.update(None, i as f64).unwrap();
            min.update(None, i as f64).unwrap();
            max.update(None, i as f64).unwrap();
        }
        let total = sum
            .estimate(&SketchQuery::PointCount {
                key: asap_types::pre_asap::ColumnRef::SampleValue,
                value: None,
            })
            .unwrap();
        assert_eq!(total, Answer::Scalar(5050.0));
        assert_eq!(
            min.estimate(&SketchQuery::Quantile { q: 0.0 }).unwrap(),
            Answer::Scalar(1.0)
        );
        assert_eq!(
            max.estimate(&SketchQuery::Quantile { q: 1.0 }).unwrap(),
            Answer::Scalar(100.0)
        );
    }

    #[test]
    fn a_keyless_family_refuses_a_key_rather_than_ignoring_it() {
        let mut handle = bind(&kll(269), NODE, 0).unwrap();
        let err = handle
            .update(Some(&ItemKey::Str("service".into())), 1.0)
            .expect_err("KLL is keyless");
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    #[test]
    fn the_kll_footprint_counts_the_merge_buffer_the_library_allocates() {
        let handle = bind(&kll(269), NODE, 0).unwrap();
        let items = kll_max_capacity(269, KLL_M) * 8;
        let levels = (KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
        let merge_buf = 269 * 8;
        assert_eq!(handle.footprint_bytes(), items + levels + merge_buf);
        assert_eq!((items, levels, merge_buf), (9648, 496, 2152));
        // Omitting the merge buffer understated the total by 17.5%, straight
        // into the denominator of the memory advantage ratio.
        let understated = merge_buf as f64 / (items + levels + merge_buf) as f64;
        assert!((understated - 0.175).abs() < 0.005, "{understated}");
    }

    #[test]
    fn an_exact_kind_answers_only_the_query_that_names_its_own_statistic() {
        let values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let total = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };
        let rows = SketchQuery::PointCount {
            key: ColumnRef::Wildcard,
            value: None,
        };

        let mut sum = bind(&exact(ExactKind::Sum, ExactParams::Sum), NODE, 0).unwrap();
        let mut count = bind(&exact(ExactKind::Count, ExactParams::Count), NODE, 0).unwrap();
        for value in &values {
            sum.update(None, *value).unwrap();
            count.update(None, *value).unwrap();
        }

        assert!(
            count.estimate(&total).is_err(),
            "Count must not answer the sum of the weights"
        );
        assert!(sum.estimate(&rows).is_err(), "Sum must not answer COUNT(*)");

        for (handle, query) in [(&mut sum, &total), (&mut count, &rows)] {
            let approximate = match handle.estimate(query).unwrap() {
                Answer::Scalar(value) => value,
                other => panic!("expected a scalar, got {other:?}"),
            };
            let exact = crate::score::exact_answer(
                &crate::types::Retained {
                    weights: values.clone(),
                    keyed: Vec::new(),
                },
                query,
            )
            .unwrap();
            assert_eq!(Answer::Scalar(approximate), exact, "{query:?}");
        }
    }

    #[test]
    fn the_ddsketch_footprint_is_the_bucket_store_the_library_allocated() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::DDSketch,
                SketchParams::DDSketch { alpha: 0.01 },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );

        let empty = bind(&family, NODE, 0).unwrap();
        assert_eq!(empty.footprint_bytes(), 0);

        let mut one_value = bind(&family, NODE, 0).unwrap();
        for _ in 0..5 {
            one_value.update(None, 42.0).unwrap();
        }
        assert_eq!(
            one_value.footprint_bytes(),
            128 * std::mem::size_of::<u64>()
        );

        let mut spread = bind(&family, NODE, 0).unwrap();
        for i in 1..=10 {
            spread.update(None, i as f64 * 10.0).unwrap();
        }
        assert_eq!(spread.footprint_bytes(), 2048);

        let mut many = bind(&family, NODE, 0).unwrap();
        for i in 0..100_000 {
            many.update(None, (i % 1000 + 1) as f64).unwrap();
        }
        assert_eq!(many.footprint_bytes(), 4096);
    }

    #[test]
    fn each_exact_kind_is_charged_for_the_one_statistic_it_reads() {
        for (kind, params) in [
            (ExactKind::Sum, ExactParams::Sum),
            (ExactKind::Count, ExactParams::Count),
            (ExactKind::Min, ExactParams::Min),
            (ExactKind::Max, ExactParams::Max),
        ] {
            let handle = bind(&exact(kind.clone(), params), NODE, 0).unwrap();
            assert_eq!(
                handle.footprint_bytes(),
                std::mem::size_of::<ExactState>(),
                "{kind:?}"
            );
        }
    }

    /// The charge above is only honest if the handle holds one accumulator,
    /// and if the charge is the whole of what it holds. Four accumulators — a
    /// `sum`, a `count`, a `min` and a `max` updated on every row — would be
    /// 40 B reported as 16, and the discriminant that selects between them is
    /// 8 of the 16; this number is the numerator of the headline memory ratio.
    #[test]
    fn an_exact_accumulator_holds_exactly_the_state_it_is_charged_for() {
        assert_eq!(
            std::mem::size_of::<ExactState>(),
            2 * std::mem::size_of::<u64>(),
            "one 8-byte accumulator plus a discriminant; a second accumulator would grow this"
        );

        for (kind, params) in [
            (ExactKind::Sum, ExactParams::Sum),
            (ExactKind::Count, ExactParams::Count),
            (ExactKind::Min, ExactParams::Min),
            (ExactKind::Max, ExactParams::Max),
        ] {
            let mut handle = ExactHandle::new(exact(kind.clone(), params), NODE, &kind);
            for value in [3.0, 1.0, 2.0] {
                handle.update(None, value).unwrap();
            }
            let held = match handle.state {
                ExactState::Sum(sum) => {
                    assert_eq!(sum, 6.0);
                    std::mem::size_of_val(&sum)
                }
                ExactState::Count(count) => {
                    assert_eq!(count, 3);
                    std::mem::size_of_val(&count)
                }
                ExactState::Min(min) => {
                    assert_eq!(min, 1.0);
                    std::mem::size_of_val(&min)
                }
                ExactState::Max(max) => {
                    assert_eq!(max, 3.0);
                    std::mem::size_of_val(&max)
                }
            };
            let discriminant = std::mem::size_of::<ExactState>() - held;
            assert_eq!(discriminant, std::mem::size_of::<u64>(), "{kind:?}");
            assert_eq!(
                handle.footprint_bytes(),
                held + discriminant,
                "{kind:?}: the reported number must be the whole of what is held"
            );
        }
    }

    /// `count(cpu_cores)` compiles to a CMS plus a bare-bucket-total readout.
    /// Every insert lands in exactly one column of every row, so row 0 sums to
    /// the total weight ingested — exactly, with no collision loss.
    #[test]
    fn a_cms_answers_the_bare_bucket_total_exactly() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let total = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };
        assert!(answers_the_same_question(&family, &total));

        let mut handle = bind(&family, NODE, 0).unwrap();
        // Far more distinct keys than columns, so the matrix is saturated with
        // collisions and a per-key estimate would be an over-count.
        for i in 0..5_000i64 {
            handle.update(Some(&ItemKey::Int(i)), 1.0).unwrap();
        }
        // Weighted inserts count for their weight, not for one apiece.
        handle.update(Some(&ItemKey::Int(0)), 7.0).unwrap();

        assert_eq!(handle.estimate(&total).unwrap(), Answer::Scalar(5_007.0));
    }

    #[test]
    fn a_count_sketch_refuses_the_insert_that_would_wrap_its_i32_counters() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch {
                    width: 272,
                    depth: 5,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let mut handle = bind(&family, NODE, 0).unwrap();
        handle
            .update(Some(&ItemKey::Int(0)), 2_000_000_000.0)
            .expect("2e9 fits in an i32 counter");
        assert!(
            handle
                .update(Some(&ItemKey::Int(0)), 2_000_000_000.0)
                .is_err(),
            "one hot key at 4e9 wraps its counters"
        );
    }

    #[test]
    fn a_cms_refuses_the_insert_that_would_wrap_its_i32_counters() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let total = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };

        let mut handle = bind(&family, NODE, 0).unwrap();
        handle
            .update(Some(&ItemKey::Int(0)), 2_000_000_000.0)
            .expect("2e9 fits in an i32 counter");
        assert!(
            handle
                .update(Some(&ItemKey::Int(1)), 2_000_000_000.0)
                .is_err(),
            "4e9 wrapped to -294967296 and published a negative count"
        );
        assert_eq!(
            handle.estimate(&total).unwrap(),
            Answer::Scalar(2_000_000_000.0),
            "the refusal must leave only what was accepted"
        );

        let mut brim = bind(&family, NODE, 0).unwrap();
        brim.update(Some(&ItemKey::Int(0)), f64::from(i32::MAX))
            .expect("i32::MAX is the largest total a single counter can hold");
        assert_eq!(
            brim.estimate(&total).unwrap(),
            Answer::Scalar(f64::from(i32::MAX))
        );
        assert!(brim.update(Some(&ItemKey::Int(0)), 1.0).is_err());
    }

    /// The per-item lookup is a different question, and a `None` value still
    /// cannot answer it.
    #[test]
    fn a_cms_still_refuses_a_named_key_with_no_value() {
        let family = SummaryFamilyType::Sketch(
            SketchKind::new(
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
            GroupingStrategy::PerSubpopulationInstance,
        );
        let named = SketchQuery::PointCount {
            key: ColumnRef::Named("item".into()),
            value: None,
        };
        assert!(!answers_the_same_question(&family, &named));

        let mut handle = bind(&family, NODE, 0).unwrap();
        handle.update(Some(&ItemKey::Str("a".into())), 1.0).unwrap();
        assert!(handle.estimate(&named).is_err());
    }
}
