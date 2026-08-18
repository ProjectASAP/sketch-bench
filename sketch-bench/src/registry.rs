//! The registry: one table naming every `(algorithm, impl)` this crate exposes
//! and what each supports, written out entry by entry in
//! [`REGISTRY`](crate::registry::REGISTRY). [`check`](crate::registry::check) is what the frontend calls.

use crate::request::Requirement;
use aqpbm_core::metrics::{is_measurable, Metric, MetricsMask, Operation, OperationMask};

pub use crate::request::{Capability, Dtype};

/// One registry entry: who a sketch is, and what it can be asked for.
pub struct SketchId {
    /// Entries sharing this answer the same question from the same knobs
    pub family: &'static str,
    pub algorithm: &'static str,
    /// The implementing library, and only that.
    pub impl_name: &'static str,
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

/// Every `(algorithm, impl)` this crate exposes. Adding one is one entry here
/// plus the wrapper it names. Entries stay grouped by family and contiguous,
/// because [`list`] breaks a group the moment `family` differs from the row above.
pub const REGISTRY: &[SketchId] = &[
    // -------- CMS (frequency) --------
    SketchId {
        family: "cms",
        algorithm: "cms",
        impl_name: "oxide",
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
        family: "cms",
        algorithm: "cms",
        impl_name: "datasketches",
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
        family: "cms",
        algorithm: "cms",
        impl_name: "polars",
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
        family: "cms",
        algorithm: "cms-fastpath-fixedmatrix",
        impl_name: "lib",
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
        family: "cms",
        algorithm: "cms-fastpath-vector2d",
        impl_name: "lib",
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
        family: "cms",
        algorithm: "cms-regularpath-vector2d",
        impl_name: "lib",
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
        family: "cms",
        algorithm: "cms-fastpath-fixedmatrix-32k-parallel",
        impl_name: "lib",
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
        family: "countsketch",
        algorithm: "countsketch",
        impl_name: "oxide",
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
        family: "countsketch",
        algorithm: "countsketch",
        impl_name: "polars",
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
        family: "countsketch",
        algorithm: "countsketch-fastpath-fixedmatrix",
        impl_name: "lib",
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
        family: "countsketch",
        algorithm: "countsketch-fastpath-vector2d",
        impl_name: "lib",
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
        family: "countsketch",
        algorithm: "countsketch-regularpath-vector2d",
        impl_name: "lib",
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
        family: "countsketch",
        algorithm: "countsketch-fastpath-fixedmatrix-32k-parallel",
        impl_name: "lib",
        description: "asap Count, FastPath, parallel insert on M5x32K",
        capability: Capability::None,
        comparator: None,
        operations: OperationMask::INSERT,
        metrics: MetricsMask::THROUGHPUT
            .union(MetricsMask::CPU)
            .union(MetricsMask::MEMORY),
    },
    // -------- HLL (cardinality) --------
    // The three `lib` precisions are one entry: one algorithm at one impl, with
    // `lg_k` the knob that moves between them.
    SketchId {
        family: "hll",
        algorithm: "hll",
        impl_name: "oxide",
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
        family: "hll",
        algorithm: "hll",
        impl_name: "datasketches",
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
        family: "hll",
        algorithm: "hll",
        impl_name: "lib",
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
        family: "hll",
        algorithm: "hll",
        impl_name: "polars",
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
    // Its own algorithm, not an impl of `hll`: the estimate is maintained on the
    // insert path instead of scanned at query time. It also supplies no `merge`,
    // which is why its operations stop at query.
    SketchId {
        family: "hll",
        algorithm: "hll-hip",
        impl_name: "lib",
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
        family: "hll",
        algorithm: "hll-fastpath-parallel",
        impl_name: "lib",
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
        family: "kll",
        algorithm: "kll-percall",
        impl_name: "oxide",
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
        family: "kll",
        algorithm: "kll-percall",
        impl_name: "lib",
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
        family: "kll",
        algorithm: "kll-cdf",
        impl_name: "oxide",
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
        family: "kll",
        algorithm: "kll-cdf",
        impl_name: "lib",
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
        family: "kll",
        algorithm: "kll-cdf",
        impl_name: "polars",
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
    // -------- Hydra (per-subpopulation statistics over labelled records) --------
    // Three families, not one: what sits in a cell decides which statistic the
    // grid answers, so each cell type gets its own params vocabulary and its own
    // comparator. See `wrappers/hydra/mod.rs`.
    SketchId {
        family: "hydra-cms",
        algorithm: "hydra-cms",
        impl_name: "lib",
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
        family: "hydra-cms",
        algorithm: "hydra-cms",
        impl_name: "polars",
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
        family: "hydra-hll",
        algorithm: "hydra-hll",
        impl_name: "lib",
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
        family: "hydra-hll",
        algorithm: "hydra-hll",
        impl_name: "polars",
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
        family: "hydra-kll",
        algorithm: "hydra-kll",
        impl_name: "lib",
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
        family: "hydra-kll",
        algorithm: "hydra-kll",
        impl_name: "polars",
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
        family: "univmon",
        algorithm: "univmon-cardinality",
        impl_name: "lib",
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
        family: "univmon",
        algorithm: "univmon-l1-norm",
        impl_name: "lib",
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
        family: "univmon",
        algorithm: "univmon-l1-norm",
        impl_name: "oxide",
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
        family: "univmon",
        algorithm: "univmon-l2-norm",
        impl_name: "lib",
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
        family: "univmon",
        algorithm: "univmon-l2-norm",
        impl_name: "oxide",
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
        family: "univmon",
        algorithm: "univmon-entropy",
        impl_name: "lib",
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
        family: "univmon",
        algorithm: "univmon-entropy",
        impl_name: "oxide",
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

pub fn find(algorithm: &str, impl_name: &str) -> Option<&'static SketchId> {
    REGISTRY
        .iter()
        .find(|r| r.algorithm == algorithm && r.impl_name == impl_name)
}

/// One line per entry, grouped by family with a blank line between groups and
/// declaration order inside one. The algorithm column is sized to the longest
/// name present; the first line is the header, so a caller prints what it gets.
pub fn list() -> Vec<String> {
    let algo_w = REGISTRY
        .iter()
        .map(|r| r.algorithm.len())
        .max()
        .unwrap_or(0)
        .max("# algorithm".len());
    let impl_w = REGISTRY
        .iter()
        .map(|r| r.impl_name.len())
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(REGISTRY.len() + 8);
    out.push(format!(
        "{:algo_w$}  {:impl_w$}  description",
        "# algorithm", "impl"
    ));
    let mut current: Option<&str> = None;
    for r in REGISTRY {
        if current != Some(r.family) {
            out.push(String::new());
            current = Some(r.family);
        }
        out.push(format!(
            "{:algo_w$}  {:impl_w$}  {}",
            r.algorithm, r.impl_name, r.description
        ));
    }
    out
}

pub fn algorithm_exists(algorithm: &str) -> bool {
    REGISTRY.iter().any(|r| r.algorithm == algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field. `None`
/// if the algorithm is unknown, which the frontend has already ruled out by the
/// time it asks.
pub fn family_of(algorithm: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|r| r.algorithm == algorithm)
        .map(|r| r.family)
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
    let entry = find(&req.algorithm, &req.impl_name).ok_or_else(|| {
        // Told apart, because they send a reader to different places: a bad
        // algorithm means look at the list, a bad impl means look at the
        // algorithm's entry in it.
        if algorithm_exists(&req.algorithm) {
            ResolveError::UnknownImpl {
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
            }
        } else {
            ResolveError::UnknownAlgorithm(req.algorithm.clone())
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
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
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
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
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
                algorithm: req.algorithm.clone(),
                impl_name: req.impl_name.clone(),
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
pub fn comparators(algorithm: &str, impl_name: &str) -> Option<Vec<&'static str>> {
    find(algorithm, impl_name).map(|entry| entry.comparator.into_iter().collect())
}

/// Why a request cannot run. Every variant names the sketch and what about the
/// request it could not honour, because the whole value of answering here is
/// that the answer arrives before a dataset is generated.
#[derive(Debug)]
pub enum ResolveError {
    UnknownAlgorithm(String),
    UnknownImpl {
        algorithm: String,
        impl_name: String,
    },
    /// An operation this entry does not have — no merge, or no prepare.
    OperationUnsupported {
        algorithm: String,
        impl_name: String,
        operation: &'static str,
        admits: String,
    },
    /// A metric this entry cannot carry — accuracy on a sketch nothing scores.
    MetricUnsupported {
        algorithm: String,
        impl_name: String,
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
        algorithm: String,
        impl_name: String,
        name: String,
        admits: String,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UnknownAlgorithm(a) => write!(f, "unknown sketch algorithm: {a}"),
            ResolveError::UnknownImpl {
                algorithm,
                impl_name,
            } => {
                write!(f, "no impl '{impl_name}' for algorithm '{algorithm}'")
            }
            ResolveError::OperationUnsupported {
                algorithm,
                impl_name,
                operation,
                admits,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} has no {operation}; it can be measured over {admits}"
                )
            }
            ResolveError::MetricUnsupported {
                algorithm,
                impl_name,
                metric,
                capability,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} answers no statistic (capability {capability}), \
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
                algorithm,
                impl_name,
                name,
                admits,
            } => {
                write!(
                    f,
                    "{algorithm}/{impl_name} has no comparator '{name}'; it admits {admits}"
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
                e.algorithm,
                e.impl_name
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
                e.algorithm,
                e.impl_name
            );
            assert_eq!(
                e.operations.contains(OperationMask::QUERY),
                e.capability.scores(),
                "{}/{} query",
                e.algorithm,
                e.impl_name
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
                    e.algorithm,
                    e.impl_name
                );
            }
        }
    }
}
