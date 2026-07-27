//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All of them declare `CardinalityOps`,
//! which is what makes them cardinality rows.

use aqpbm_core::accuracy::CardinalityOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::HllParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::{MergeUnsupported, Sketch};
// sketch_oxide routes `.estimate()` through its `Sketch` trait.
use sketch_oxide::Sketch as OxideSketch;
// `merge` lives on sketch_oxide's `Mergeable`, not on its `Sketch`.
use sketch_oxide::Mergeable as _;

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

impl InitSketch for HllOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
            .map_err(|e| BuildError(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
        Ok(Self {
            inner,
            lg_k: p.lg_k,
        })
    }
}

impl Sketch for HllOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn memory_bytes(&self) -> usize {
        // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
        1usize << self.lg_k
    }

    /// Register-wise max. Exact for equal `lg_k`: the merged sketch is
    /// bit-identical to one fed the whole stream, so any error the merge
    /// benchmark reports beyond the single-pass figure is a real defect.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so lg_k matches");
        Ok(())
    }
}

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: datasketches::hll::HllType,
}

impl InitSketch for HllDatasketches {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        let hll_type = datasketches::hll::HllType::Hll8;
        Ok(Self {
            inner: datasketches::hll::HllSketch::new(p.lg_k, hll_type),
            lg_k: p.lg_k,
            hll_type,
        })
    }
}

impl Sketch for HllDatasketches {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }
    fn memory_bytes(&self) -> usize {
        // Apache datasketches packs registers per HllType:
        // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
        let m = 1usize << self.lg_k;
        match self.hll_type {
            datasketches::hll::HllType::Hll4 => m / 2,
            datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
            datasketches::hll::HllType::Hll8 => m,
        }
    }

    /// Apache DataSketches routes HLL merging through a `Union` gadget rather
    /// than a method on the sketch, so this rebuilds `self` from the union's
    /// result.
    ///
    /// **Known limitation, unresolved.** Every other HLL row folds registers
    /// in place and the merge benchmark reports it lossless, as theory
    /// requires for equal `lg_k`. This row reports *lossy*, and its
    /// `merge_time_ms` runs ~2x the others. Both are plausibly artifacts of
    /// this wrapper rather than of the library: a pairwise `merge` signature
    /// forces a fresh `HllUnion` and a `get_result` representation round-trip
    /// on **every** fold, all inside the timed region. Fixing it properly
    /// needs a fold-shaped hook (`merge_many`) so one union spans the whole
    /// fold. Until then this row's merge numbers say nothing about
    /// DataSketches and should not be compared against the others.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        let mut union = datasketches::hll::HllUnion::new(self.lg_k);
        union.update(&self.inner);
        union.update(&other.inner);
        self.inner = union.get_result(self.hll_type);
        Ok(())
    }
}

// ---------- asap_sketchlib HLL ----------
// `asap_sketchlib::HyperLogLog<Classic>` is the P14 classic HLL estimator
// (Flajolet et al., 2007). Insert path only bumps registers; `estimate()`
// scans all 2^14 registers (O(m)). Compile-time fixed at P14; the
// requested `lg_k` is ignored.
pub struct HllLib {
    inner: asap_sketchlib::HyperLogLog<asap_sketchlib::Classic>,
}

impl InitSketch for HllLib {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let _p: HllParams = config.parse()?;
        Ok(Self {
            inner: asap_sketchlib::HyperLogLog::<asap_sketchlib::Classic>::new(),
        })
    }
}

impl Sketch for HllLib {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        // Implementation is fixed at P14 — register count is 2^14
        // regardless of `HllParams::lg_k`, 1 byte per register.
        1usize << 14
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

// `asap_sketchlib::HyperLogLogHIP` (= HyperLogLogHIPP14) maintains the
// cardinality estimate incrementally on the insert path — every register
// upgrade pays a handful of fp ops on the running `est` / `kxq0` / `kxq1`
// fields — so query is O(1) rather than the Classic variant's O(m) register
// scan. That trade (slightly slower inserts, query throughput on par with
// apache DataSketches' `get_estimate`) is what this row exists to measure.
pub struct HllLibHip {
    inner: asap_sketchlib::HyperLogLogHIP,
}

impl InitSketch for HllLibHip {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let _p: HllParams = config.parse()?;
        Ok(Self {
            inner: asap_sketchlib::HyperLogLogHIP::new(),
        })
    }
}

impl Sketch for HllLibHip {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        1usize << 14
    }
}


// ---------- statistic membership ----------
//
// The parallel-HLL row ingests `i64` just like these do but answers nothing,
// so it is absent here — which is what keeps it out of accuracy scoring.

impl CardinalityOps for HllOxide {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl CardinalityOps for HllDatasketches {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl CardinalityOps for HllLib {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

impl CardinalityOps for HllLibHip {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

// ---------- catalog identity ----------

impl BenchImpl for HllOxide { type Params = HllParams; const IMPL: &'static str = "oxide"; }
impl BenchImpl for HllDatasketches { type Params = HllParams; const IMPL: &'static str = "datasketches"; }
impl BenchImpl for HllLib { type Params = HllParams; const IMPL: &'static str = "lib"; }
impl BenchImpl for HllLibHip { type Params = HllParams; const IMPL: &'static str = "lib-hip"; }
