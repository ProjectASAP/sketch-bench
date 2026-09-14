//! SBT-1 — bind a planner-chosen summary family to a concrete constructor, or
//! refuse it by name.
//!
//! v0 binds exactly what the corpus produces: `Kll{k}` and the three exact
//! accumulators. Every other family is refused. Binding an algorithm the
//! corpus does not exercise would mean carrying an untested estimator whose
//! wrong answers look plausible — and a substitution (answering `Theta` with
//! HLL, say) would leave the readout's `ResultGuarantee` describing an
//! algorithm that did not run, voiding the accuracy claim while every number
//! still looked reasonable.

use asap_sketchlib::KLL;
use asap_types::post_asap::{
    ExactKind, ExactParams, GroupingStrategy, PostAsapNodeId, SketchAlgorithm, SketchCategory,
    SketchParams, SketchQuery, SummaryFamilyType,
};

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
const KLL_MAX_LEVELS: usize = 61;
/// `CAPACITY_DECAY` (`kll.rs:30`).
const KLL_CAPACITY_DECAY: f64 = 2.0 / 3.0;
/// The `m` that `init_kll_with_seed` passes to `init` (`kll.rs:291-298`).
const KLL_M: usize = 8;

/// Replica of the private `compute_max_capacity` (`kll.rs:119-127`). A replica
/// rather than a `k * 4` approximation because the reported footprint is the
/// denominator of this crate's headline ratio.
fn kll_max_capacity(k: usize, m: usize) -> usize {
    let mut total = 0usize;
    let mut scale = 1.0_f64;
    for _ in 0..KLL_MAX_LEVELS {
        total += ((k as f64) * scale).ceil().max(m as f64) as usize;
        scale *= KLL_CAPACITY_DECAY;
    }
    total
}

/// One live summary instance, keyed elsewhere by `StateKey{plan, node, group}`.
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
                (SketchCategory::Quantile, SketchAlgorithm::Kll, SketchParams::Kll { k }) => {
                    if !(KLL_K_MIN..=KLL_K_MAX).contains(k) {
                        return Err(Refusal::ParameterOutOfBounds {
                            node,
                            detail: format!(
                                "Kll k = {k} is outside [{KLL_K_MIN}, {KLL_K_MAX}]; the library \
                                 would clamp it silently and the recorded k would not be the k \
                                 that ran"
                            ),
                        });
                    }
                    Ok(())
                }
                (category, algorithm, params)
                    if !params_match_algorithm(algorithm, params)
                        || !category_matches_algorithm(&category, algorithm) =>
                {
                    Err(Refusal::InconsistentSketchKind {
                        node,
                        detail: format!("{category:?} / {algorithm:?} / {params:?}"),
                    })
                }
                // Internally consistent, but v0 binds no other algorithm.
                _ => Err(Refusal::UnboundFamily {
                    node,
                    family: Box::new(SummaryFamilyType::Sketch(kind.clone(), grouping.clone())),
                }),
            }
        }
        SummaryFamilyType::ExactAggregate(kind, params) => match (kind, params) {
            (ExactKind::Sum, ExactParams::Sum)
            | (ExactKind::Count, ExactParams::Count)
            | (ExactKind::MinMax, ExactParams::MinMax) => Ok(()),
            // Increase/Rate/IRate hold state that depends on sample order and
            // window duration, and neither reaches `update(item, weight)`.
            (ExactKind::Increase, ExactParams::Increase)
            | (ExactKind::Rate, ExactParams::Rate)
            | (ExactKind::IRate, ExactParams::IRate) => Err(Refusal::UnboundFamily {
                node,
                family: Box::new(family.clone()),
            }),
            (kind, params) => Err(Refusal::InconsistentSketchKind {
                node,
                detail: format!("ExactAggregate({kind:?}, {params:?})"),
            }),
        },
        other => Err(Refusal::UnboundFamily {
            node,
            family: Box::new(other.clone()),
        }),
    }
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
            | (SketchAlgorithm::CountSketch, SketchParams::CountSketch { .. })
            | (SketchAlgorithm::CmsWithHeap, SketchParams::CmsWithHeap { .. })
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
            _ => unreachable!("check_bindable admits only Kll"),
        },
        SummaryFamilyType::ExactAggregate(kind, _) => {
            Ok(Box::new(ExactHandle::new(family.clone(), node, kind)))
        }
        _ => unreachable!("check_bindable admits only Sketch and ExactAggregate"),
    }
}

fn reject_item(node: PostAsapNodeId, item: Option<&ItemKey>, family: &str) -> Result<(), EvalError> {
    match item {
        None => Ok(()),
        Some(_) => Err(EvalError::Refused(vec![Refusal::UnsupportedUpdate {
            node,
            detail: format!("{family} is keyless; `input.item` must be absent"),
        }])),
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

/// Sum / Count / MinMax. No library involved, and the only handles for which an
/// inverse operation could ever be implemented honestly.
struct ExactHandle {
    family: SummaryFamilyType,
    node: PostAsapNodeId,
    kind: ExactKind,
    sum: f64,
    count: u64,
    min: f64,
    max: f64,
}

impl ExactHandle {
    fn new(family: SummaryFamilyType, node: PostAsapNodeId, kind: &ExactKind) -> Self {
        Self {
            family,
            node,
            kind: kind.clone(),
            sum: 0.0,
            count: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl SummaryHandle for ExactHandle {
    fn update(&mut self, item: Option<&ItemKey>, weight: f64) -> Result<(), EvalError> {
        reject_item(self.node, item, "an exact accumulator")?;
        self.sum += weight;
        self.count += 1;
        self.min = self.min.min(weight);
        self.max = self.max.max(weight);
        Ok(())
    }

    /// An exact accumulator's state *is* its value, so the corpus reaches it
    /// through `FinalizeExactAccumulator` rather than a `SummaryEstimate`. This
    /// arm exists for a caller that asks anyway; the mapping is our convention,
    /// not something the IR specifies.
    fn estimate(&mut self, query: &SketchQuery) -> Result<Answer, EvalError> {
        match (&self.kind, query) {
            (ExactKind::Sum, SketchQuery::PointCount { .. }) => Ok(Answer::Scalar(self.sum)),
            (ExactKind::Count, SketchQuery::PointCount { .. }) => {
                Ok(Answer::Scalar(self.count as f64))
            }
            // `Answer::Scalar` holds one number and MinMax holds two, so only
            // the endpoints are answerable.
            (ExactKind::MinMax, SketchQuery::Quantile { q }) if *q <= 0.0 => {
                Ok(Answer::Scalar(self.min))
            }
            (ExactKind::MinMax, SketchQuery::Quantile { q }) if *q >= 1.0 => {
                Ok(Answer::Scalar(self.max))
            }
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
        std::mem::size_of::<f64>() * 3 + std::mem::size_of::<u64>()
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
    fn every_family_v0_does_not_bind_is_refused_by_name_never_substituted() {
        for (algorithm, params) in [
            (SketchAlgorithm::Theta, SketchParams::Theta { k: 4096 }),
            (SketchAlgorithm::Kmv, SketchParams::Kmv { k: 1_000_002 }),
            (SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 }),
            (
                SketchAlgorithm::Cms,
                SketchParams::Cms {
                    width: 272,
                    depth: 5,
                },
            ),
        ] {
            let family = SummaryFamilyType::Sketch(
                // `new` derives the category, so these triples are internally
                // consistent — they are refused for being unbound, not malformed.
                SketchKind::new(algorithm, params),
                GroupingStrategy::PerSubpopulationInstance,
            );
            assert!(
                matches!(
                    check_bindable(&family, NODE),
                    Err(Refusal::UnboundFamily { .. })
                ),
                "{family:?} must be refused, never answered by a stand-in"
            );
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
        let family =
            SummaryFamilyType::Sketch(kind, GroupingStrategy::PerSubpopulationInstance);
        assert!(matches!(
            check_bindable(&family, NODE),
            Err(Refusal::InconsistentSketchKind { .. })
        ));
    }

    #[test]
    fn exact_accumulators_are_exact() {
        let mut sum = bind(&exact(ExactKind::Sum, ExactParams::Sum), NODE, 0).unwrap();
        let mut minmax = bind(&exact(ExactKind::MinMax, ExactParams::MinMax), NODE, 0).unwrap();
        for i in 1..=100 {
            sum.update(None, i as f64).unwrap();
            minmax.update(None, i as f64).unwrap();
        }
        let total = sum
            .estimate(&SketchQuery::PointCount {
                key: asap_types::pre_asap::ColumnRef::SampleValue,
                value: None,
            })
            .unwrap();
        assert_eq!(total, Answer::Scalar(5050.0));
        assert_eq!(
            minmax.estimate(&SketchQuery::Quantile { q: 0.0 }).unwrap(),
            Answer::Scalar(1.0)
        );
        assert_eq!(
            minmax.estimate(&SketchQuery::Quantile { q: 1.0 }).unwrap(),
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
}
