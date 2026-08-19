//! The registry: one table naming every `(variance, library)` this crate exposes
//! and what each supports, written out entry by entry in
//! [`REGISTRY`](crate::registry::REGISTRY). [`check`](crate::registry::check) is what the frontend calls.

use crate::request::Requirement;
use aqpbm_core::metrics::{is_measurable, Metric, MetricsMask, Operation, OperationMask};

pub use crate::request::{Capability, Dtype};

/// One registry entry: who a sketch is, and what it can be asked for.
pub struct SketchId {
    /// Entries sharing this answer the same question from the same knobs
    pub algorithm: &'static str,
    pub variance: &'static str,
    /// The implementing library, and only that.
    pub library: &'static str,
    pub description: &'static str,
    /// The statistic this sketch answers
    pub capability: Capability,
    /// The comparator `--comparator` selects this sketch by
    pub comparator: Option<&'static str>,
    /// The operations this sketch can be measured over
    pub operations: OperationMask,
    /// The metrics this sketch can carry. Everything but accuracy, which needs
    /// a comparator and so follows [`SketchId::capability`].
    pub metrics: MetricsMask,
}

// ---------- the registry ----------

/// Every `(variance, library)` this crate exposes. Adding one is one entry here
/// plus the wrapper it names. Entries stay grouped by algorithm and contiguous,
/// because [`list`] breaks a group the moment `algorithm` differs from the row above.
pub const REGISTRY: &[SketchId] = &[
    // -------- CMS (frequency) --------
    SketchId {
        algorithm: "cms",
        variance: "cms",
        library: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "cms",
        variance: "cms",
        library: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "cms",
        variance: "cms",
        library: "polars",
        description: "polars exact: group_by(v).agg(len)",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "cms",
        variance: "cms-fastpath-fixedmatrix",
        library: "lib",
        description: "asap CMS, FixedMatrix (shape baked at compile time), FastPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "cms",
        variance: "cms-fastpath-vector2d",
        library: "lib",
        description: "asap CMS, Vector2D, FastPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "cms",
        variance: "cms-regularpath-vector2d",
        library: "lib",
        description: "asap CMS, Vector2D, RegularPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    // Parallel insert: no `ask`, so nothing scores it and no query is offered.
    SketchId {
        algorithm: "cms",
        variance: "cms-fastpath-fixedmatrix-32k-parallel",
        library: "lib",
        description: "asap CMS, FastPath, parallel insert on M5x32K",
        capability: Capability::None,
        comparator: None,
        operations: OperationMask::INSERT,
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY),
    },
    // -------- CountSketch (frequency) --------
    // No `datasketches` entry: that library ships no CountSketch.
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch",
        library: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch",
        library: "polars",
        description: "polars exact: group_by(v).agg(len)",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch-fastpath-fixedmatrix",
        library: "lib",
        description: "asap Count, FixedMatrix (shape baked at compile time), FastPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch-fastpath-vector2d",
        library: "lib",
        description: "asap Count, Vector2D, FastPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch-regularpath-vector2d",
        library: "lib",
        description: "asap Count, Vector2D, RegularPath",
        capability: Capability::Frequency,
        comparator: Some("frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "countsketch",
        variance: "countsketch-fastpath-fixedmatrix-32k-parallel",
        library: "lib",
        description: "asap Count, FastPath, parallel insert on M5x32K",
        capability: Capability::None,
        comparator: None,
        operations: OperationMask::INSERT,
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY),
    },
    // -------- HLL (cardinality) --------
    // The three `lib` precisions are one entry: one variance at one library, with
    // `lg_k` the knob that moves between them.
    SketchId {
        algorithm: "hll",
        variance: "hll",
        library: "oxide",
        description: "sketch_oxide::cardinality::HyperLogLog (lg_k 4..=18)",
        capability: Capability::Cardinality,
        comparator: Some("cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hll",
        variance: "hll",
        library: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8)",
        capability: Capability::Cardinality,
        comparator: Some("cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hll",
        variance: "hll",
        library: "lib",
        description: "asap_sketchlib::HyperLogLog<Classic>: O(m) estimate, lg_k in {12,14,16}",
        capability: Capability::Cardinality,
        comparator: Some("cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hll",
        variance: "hll",
        library: "polars",
        description: "polars exact: DataFrame.n_unique()",
        capability: Capability::Cardinality,
        comparator: Some("cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    // Its own variance, not an impl of `hll`: the estimate is maintained on the
    // insert path instead of scanned at query time. It also supplies no `merge`,
    // which is why its operations stop at query.
    SketchId {
        algorithm: "hll",
        variance: "hll-hip",
        library: "lib",
        description: "asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}",
        capability: Capability::Cardinality,
        comparator: Some("cardinality"),
        operations: OperationMask::INSERT.union(OperationMask::QUERY),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hll",
        variance: "hll-fastpath-parallel",
        library: "lib",
        description: "asap HLL ErtlMLE, FastPath, parallel insert",
        capability: Capability::None,
        comparator: None,
        operations: OperationMask::INSERT,
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY),
    },
    // -------- KLL (quantile) --------
    // Two query paths x two libraries. The `cdf` entries supply a `prepare` and
    // the per-call ones do not, which is the whole point of the split. These are
    // also the only entries that build at f64 as well as i64.
    SketchId {
        algorithm: "kll",
        variance: "kll-percall",
        library: "oxide",
        description: "sketch_oxide KllSketch: quantile() per call",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "kll",
        variance: "kll-percall",
        library: "lib",
        description: "asap_sketchlib::KLL: quantile() per call, k in [8, 26602]",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "kll",
        variance: "kll-cdf",
        library: "oxide",
        description: "sketch_oxide KllSketch: cdf() built in prepare",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "kll",
        variance: "kll-cdf",
        library: "lib",
        description: "asap_sketchlib::KLL: cdf() built in prepare, k in [8, 26602]",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    // The exact baseline is i64 only, unlike the four sketch entries above it.
    SketchId {
        algorithm: "kll",
        variance: "kll-cdf",
        library: "polars",
        description: "polars exact: 101-point quantile grid",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    // -------- DDSketch (quantile) --------
    SketchId {
        algorithm: "dd",
        variance: "dd",
        library: "lib",
        description: "asap_sketchlib::DDSketch: relative-error buckets, alpha in (0, 1)",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "dd",
        variance: "dd",
        library: "oxide",
        description: "sketch_oxide::quantiles::DDSketch: relative-error buckets, alpha in (0, 1)",
        capability: Capability::Quantile,
        comparator: Some("rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    // -------- Hydra (per-subpopulation statistics over labelled records) --------
    // Three families, not one: what sits in a cell decides which statistic the
    // grid answers, so each cell type gets its own params vocabulary and its own
    // comparator. See `wrappers/hydra/mod.rs`.
    SketchId {
        algorithm: "hydra-cms",
        variance: "hydra-cms",
        library: "lib",
        description: "asap_sketchlib::Hydra over Count-Min cells (subpopulation frequency)",
        capability: Capability::SubpopFrequency,
        comparator: Some("subpop-frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hydra-cms",
        variance: "hydra-cms",
        library: "polars",
        description: "polars exact: group_by(subset, v).agg(len) over every label subset",
        capability: Capability::SubpopFrequency,
        comparator: Some("subpop-frequency"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hydra-hll",
        variance: "hydra-hll",
        library: "lib",
        description: "asap_sketchlib::Hydra over HyperLogLog cells (subpopulation cardinality)",
        capability: Capability::SubpopCardinality,
        comparator: Some("subpop-cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hydra-hll",
        variance: "hydra-hll",
        library: "polars",
        description: "polars exact: group_by(subset).agg(v.n_unique()) over every label subset",
        capability: Capability::SubpopCardinality,
        comparator: Some("subpop-cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hydra-kll",
        variance: "hydra-kll",
        library: "lib",
        description: "asap_sketchlib::Hydra over KLL cells (subpopulation quantile)",
        capability: Capability::SubpopQuantile,
        comparator: Some("subpop-rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "hydra-kll",
        variance: "hydra-kll",
        library: "polars",
        description: "polars exact: sorted values per label subset, quantile by rank",
        capability: Capability::SubpopQuantile,
        comparator: Some("subpop-rank-error"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::PREPARE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-cardinality",
        library: "lib",
        description: "asap_sketchlib::UnivMon: calc_card, the keys carrying a non-zero total",
        capability: Capability::KeyedCardinality,
        comparator: Some("keyed-cardinality"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-l1-norm",
        library: "lib",
        description: "asap_sketchlib::UnivMon: calc_l1, the sum of the per-key totals",
        capability: Capability::KeyedL1Norm,
        comparator: Some("keyed-l1-norm"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-l1-norm",
        library: "oxide",
        description: "sketch_oxide::universal::UnivMon: estimate_l1, summed on the insert path",
        capability: Capability::KeyedL1Norm,
        comparator: Some("keyed-l1-norm"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-l2-norm",
        library: "lib",
        description: "asap_sketchlib::UnivMon: calc_l2, the root of the summed squared totals",
        capability: Capability::KeyedL2Norm,
        comparator: Some("keyed-l2-norm"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-l2-norm",
        library: "oxide",
        description: "sketch_oxide::universal::UnivMon: estimate_l2, a Count Sketch self product",
        capability: Capability::KeyedL2Norm,
        comparator: Some("keyed-l2-norm"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-entropy",
        library: "lib",
        description: "asap_sketchlib::UnivMon: calc_entropy, Shannon entropy of the key shares",
        capability: Capability::KeyedEntropy,
        comparator: Some("keyed-entropy"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
    SketchId {
        algorithm: "univmon",
        variance: "univmon-entropy",
        library: "oxide",
        description: "sketch_oxide::universal::UnivMon: estimate_entropy, over its sampled layers",
        capability: Capability::KeyedEntropy,
        comparator: Some("keyed-entropy"),
        operations: OperationMask::INSERT
            .union(OperationMask::QUERY)
            .union(OperationMask::MERGE),
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::LATENCY)
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY)
            .union(MetricsMask::ACCURACY),
    },
];

// ---------- what the frontend asks ----------

pub fn find(variance: &str, library: &str) -> Option<&'static SketchId> {
    REGISTRY
        .iter()
        .find(|r| r.variance == variance && r.library == library)
}

/// One line per entry, grouped by algorithm with a blank line between groups and
/// declaration order inside one. The variance column is sized to the longest
/// name present; the first line is the header, so a caller prints what it gets.
pub fn list() -> Vec<String> {
    let algo_w = REGISTRY
        .iter()
        .map(|r| r.variance.len())
        .max()
        .unwrap_or(0)
        .max("# variance".len());
    let impl_w = REGISTRY.iter().map(|r| r.library.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(REGISTRY.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  description",
        "# variance", "library"
    ));
    let mut current: Option<&str> = None;
    for r in REGISTRY {
        if current != Some(r.algorithm) {
            out.push(String::new());
            current = Some(r.algorithm);
        }
        out.push(format!(
            "{:algo_w$}  {:impl_w$}  {}",
            r.variance, r.library, r.description
        ));
    }
    out
}

pub fn variance_exists(variance: &str) -> bool {
    REGISTRY.iter().any(|r| r.variance == variance)
}

/// The algorithm a variance belongs to, for the record's `algorithm` field. `None`
/// if the variance is unknown, which the frontend has already ruled out by the
/// time it asks.
pub fn algorithm_of(variance: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|r| r.variance == variance)
        .map(|r| r.algorithm)
}

// ---------- can this request run? ----------

/// Can this request run? Every check is answerable from the request, the
/// measurements it intends to take, and [`REGISTRY`] alone, so a refusal lands
/// before a single item is generated. Construction parameters are *not*
/// checked — a wrapper owns those bounds.
///
/// `want` is the caller's whole list, checked up front: a request is refused
/// as a unit rather than part-way through measuring it.
pub fn check(req: &Requirement, want: &[(Operation, Metric)]) -> Result<(), ResolveError> {
    let entry = find(&req.variance, &req.library).ok_or_else(|| {
        // Told apart, because they send a reader to different places: a bad
        // variance means look at the list, a bad library means look at the
        // variance's entry in it.
        if variance_exists(&req.variance) {
            ResolveError::UnknownLibrary {
                variance: req.variance.clone(),
                library: req.library.clone(),
            }
        } else {
            ResolveError::UnknownVariance(req.variance.clone())
        }
    })?;

    // Every measurement asked for has to clear two independent bars: the
    // framework has to measure it at all, and this entry has to have it. The
    // framework's bar is checked first, because it holds of every sketch and so
    // is not this entry's fault.
    for (operation, metric) in want {
        if !is_measurable(*operation, *metric) {
            return Err(ResolveError::NothingMeasuresIt {
                operation: operation.name(),
                metric: metric.name(),
            });
        }
    }

    // The two axes the entry declares against, folded back out of the list.
    let wants_metric = want
        .iter()
        .fold(MetricsMask::empty(), |m, (_, metric)| m | metric.bit());
    let wants_operation = want
        .iter()
        .fold(OperationMask::empty(), |m, (operation, _)| {
            m | operation.bit()
        });

    // Metrics before operations, deliberately. A sketch nothing scores lacks
    // accuracy *and* query, so checking operations first would always answer
    // "no query" and never name the capability that is the actual reason.
    for (bit, metric) in [
        (MetricsMask::ACCURACY, "accuracy"),
        (MetricsMask::THROUGHPUT, "throughput"),
        (MetricsMask::LATENCY, "latency"),
    ] {
        if wants_metric.contains(bit) && !entry.metrics.contains(bit) {
            return Err(ResolveError::MetricUnsupported {
                variance: req.variance.clone(),
                library: req.library.clone(),
                metric,
                capability: entry.capability.name(),
            });
        }
    }

    for (bit, operation) in [
        (OperationMask::INSERT, "insert"),
        (OperationMask::QUERY, "query"),
        (OperationMask::MERGE, "merge"),
        (OperationMask::PREPARE, "prepare"),
    ] {
        if wants_operation.contains(bit) && !entry.operations.contains(bit) {
            return Err(ResolveError::OperationUnsupported {
                variance: req.variance.clone(),
                library: req.library.clone(),
                operation,
                admits: operations_of(entry),
            });
        }
    }

    // A named comparator has to be the one this entry is scored by. Omitted
    // takes the entry's own, so `None` asks nothing.
    if let Some(name) = req.comparator.as_deref() {
        if entry.comparator != Some(name) {
            return Err(ResolveError::UnknownComparator {
                variance: req.variance.clone(),
                library: req.library.clone(),
                name: name.to_string(),
                admits: comparators_of(entry),
            });
        }
    }

    // An empty mask on either axis names no measurements. Legal, and not this
    // function's business: the caller gets no records because it asked for
    // none, not because anything failed.
    Ok(())
}

/// The operations an entry admits, for an error message.
fn operations_of(entry: &SketchId) -> String {
    let names: Vec<&str> = [
        (OperationMask::INSERT, "insert"),
        (OperationMask::QUERY, "query"),
        (OperationMask::MERGE, "merge"),
        (OperationMask::PREPARE, "prepare"),
    ]
    .into_iter()
    .filter(|(b, _)| entry.operations.contains(*b))
    .map(|(_, n)| n)
    .collect();
    names.join(", ")
}

/// The comparator names an entry admits, for an error message.
fn comparators_of(entry: &SketchId) -> String {
    entry.comparator.unwrap_or("none").to_string()
}

/// Which comparators an entry admits. `None` for an unknown entry, so a
/// frontend can tell "no such sketch" from "that sketch is scored by nothing".
pub fn comparators(variance: &str, library: &str) -> Option<Vec<&'static str>> {
    find(variance, library).map(|entry| entry.comparator.into_iter().collect())
}

/// Why a request cannot run. Every variant names the sketch and what about the
/// request it could not honour, because the whole value of answering here is
/// that the answer arrives before a dataset is generated.
#[derive(Debug)]
pub enum ResolveError {
    UnknownVariance(String),
    UnknownLibrary {
        variance: String,
        library: String,
    },
    /// An operation this entry does not have — no merge, or no prepare.
    OperationUnsupported {
        variance: String,
        library: String,
        operation: &'static str,
        admits: String,
    },
    /// A metric this entry cannot carry — accuracy on a sketch nothing scores.
    MetricUnsupported {
        variance: String,
        library: String,
        metric: &'static str,
        capability: &'static str,
    },
    /// A measurement no sketch could supply, because the framework measures nothing
    /// there. Distinct from the two above: this is not about the sketch.
    NothingMeasuresIt {
        operation: &'static str,
        metric: &'static str,
    },
    UnknownComparator {
        variance: String,
        library: String,
        name: String,
        admits: String,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UnknownVariance(a) => write!(f, "unknown sketch variance: {a}"),
            ResolveError::UnknownLibrary { variance, library } => {
                write!(f, "no library '{library}' for variance '{variance}'")
            }
            ResolveError::OperationUnsupported {
                variance,
                library,
                operation,
                admits,
            } => {
                write!(
                    f,
                    "{variance}/{library} has no {operation}; it can be measured over {admits}"
                )
            }
            ResolveError::MetricUnsupported {
                variance,
                library,
                metric,
                capability,
            } => {
                write!(
                    f,
                    "{variance}/{library} answers no statistic (capability {capability}), \
                     so nothing can score its {metric}"
                )
            }
            ResolveError::NothingMeasuresIt { operation, metric } => {
                write!(
                    f,
                    "nothing measures the {metric} of {operation}, for any sketch"
                )
            }
            ResolveError::UnknownComparator {
                variance,
                library,
                name,
                admits,
            } => {
                write!(
                    f,
                    "{variance}/{library} has no comparator '{name}'; it admits {admits}"
                )
            }
        }
    }
}

impl std::error::Error for ResolveError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Accuracy is declared exactly where a comparator can score it.
    #[test]
    fn accuracy_is_declared_exactly_where_a_comparator_can_score_it() {
        for e in REGISTRY {
            assert_eq!(
                e.metrics.contains(MetricsMask::ACCURACY),
                e.capability.scores(),
                "{}/{}",
                e.variance,
                e.library
            );
        }
    }

    /// A comparator and a query go together with the capability, for the same
    /// reason: a sketch nothing scores is asked nothing.
    #[test]
    fn a_comparator_and_a_query_follow_the_capability() {
        for e in REGISTRY {
            assert_eq!(
                e.comparator.is_some(),
                e.capability.scores(),
                "{}/{} comparator",
                e.variance,
                e.library
            );
            assert_eq!(
                e.operations.contains(OperationMask::QUERY),
                e.capability.scores(),
                "{}/{} query",
                e.variance,
                e.library
            );
        }
    }

    /// Insert holds of everything, and throughput and the footprint metrics hold
    /// of everything. Nothing in the table is measured over nothing. Latency is
    /// not in the floor: a row that ingests the whole stream in one call has no
    /// per-item region to time, and says so by not claiming the metric.
    #[test]
    fn every_entry_is_at_least_a_timed_insert() {
        for e in REGISTRY {
            assert!(e.operations.contains(OperationMask::INSERT));
            assert!(e.metrics.contains(
                MetricsMask::THROUGHPUT
                    .union(MetricsMask::CPU)
                    .union(MetricsMask::MEMORY)
            ));
        }
    }

    /// Every declared comparator is one `aqpbm-core` actually ships, spelled the
    /// way `--comparator` takes it.
    #[test]
    fn every_declared_comparator_is_one_core_ships() {
        const KNOWN: &[&str] = &[
            "cardinality",
            "frequency",
            "rank-error",
            "subpop-cardinality",
            "subpop-frequency",
            "subpop-rank-error",
            "keyed-cardinality",
            "keyed-l1-norm",
            "keyed-l2-norm",
            "keyed-entropy",
        ];
        for e in REGISTRY {
            if let Some(name) = e.comparator {
                assert!(
                    KNOWN.contains(&name),
                    "{}/{}: {name}",
                    e.variance,
                    e.library
                );
            }
        }
    }
}
