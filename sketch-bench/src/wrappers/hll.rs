//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All of them declare `CardinalityOps`,
//! which is what makes them cardinality rows.

use aqpbm_core::accuracy::CardinalityOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::HllParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
// sketch_oxide routes `.estimate()` through its `Accumulator` trait.
use sketch_oxide::Sketch as OxideSketch; // NOTE: foreign trait, not ours
// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.
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

impl Accumulator for HllOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
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

impl MemoryFootprint for HllOxide {
    fn memory_bytes(&self) -> usize {
        // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
        1usize << self.lg_k
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

impl Accumulator for HllDatasketches {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }

    /// DataSketches merges HLL through a `Union` gadget, so this rebuilds `self`
    /// from its result. **Known limitation:** a fresh `HllUnion` per fold inside
    /// the timed region makes this row read lossy and ~2x slow — not comparable.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        let mut union = datasketches::hll::HllUnion::new(self.lg_k);
        union.update(&self.inner);
        union.update(&other.inner);
        self.inner = union.get_result(self.hll_type);
        Ok(())
    }
}

impl MemoryFootprint for HllDatasketches {
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
}

// ---------- asap_sketchlib HLL ----------
// `HyperLogLog<Classic>`, the P14 classic estimator (Flajolet et al., 2007):
// insert bumps registers, `estimate()` scans 2^14. Fixed at P14; `lg_k` inert.
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

impl Accumulator for HllLib {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for HllLib {
    fn memory_bytes(&self) -> usize {
        // Implementation is fixed at P14 — register count is 2^14
        // regardless of `HllParams::lg_k`, 1 byte per register.
        1usize << 14
    }
}

// `HyperLogLogHIP` maintains the estimate incrementally on the insert path —
// every register upgrade pays a few fp ops — so query is O(1) rather than
// Classic's O(m) scan. That trade is what this row exists to measure.
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

impl Accumulator for HllLibHip {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
}

impl MemoryFootprint for HllLibHip {
    fn memory_bytes(&self) -> usize {
        1usize << 14
    }
}


// ---------- statistic membership ----------
// The parallel-HLL row ingests `i64` like these but answers nothing, so it is
// absent here — which is what keeps it out of accuracy scoring.

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
