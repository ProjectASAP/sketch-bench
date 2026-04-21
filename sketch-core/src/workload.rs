//! Synthetic workload generators + adapters for file-backed
//! test data.
//!
//! See `docs/DESIGN.md` §4.3.

use rand::SeedableRng;
use rand_distr::{Distribution, Uniform, Zipf};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SketchCoreError;

/// Human-friendly description of a workload — serialised into
/// every report so a JSONL record can be re-run without
/// external metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadDesc {
    pub shape: String, // "uniform" | "zipf" | "file"
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zipf_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// The abstract contract for a workload a `BenchRunner` can
/// consume. An implementation produces an ordered `Vec<Item>`
/// plus an optional query stream.
pub trait Workload {
    type Item: Clone;
    fn desc(&self) -> WorkloadDesc;
    fn items(&self) -> &[Self::Item];
}

// ---------- i64 workloads ----------

/// Uniform `i64` workload in `[0, cardinality)`.
#[derive(Debug, Clone)]
pub struct UniformI64 {
    items: Vec<i64>,
    cardinality: u64,
    seed: u64,
}

impl UniformI64 {
    pub fn new(size: usize, cardinality: u64, seed: u64) -> Self {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let dist = Uniform::new(0u64, cardinality);
        let items = (0..size).map(|_| dist.sample(&mut rng) as i64).collect();
        Self {
            items,
            cardinality,
            seed,
        }
    }
}

impl Workload for UniformI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "uniform".into(),
            size: self.items.len(),
            cardinality: Some(self.cardinality),
            zipf_s: None,
            source_path: None,
            seed: Some(self.seed),
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

/// Zipfian `i64` workload with `s`-parameter (skew exponent)
/// over keys `[1, cardinality]`.
#[derive(Debug, Clone)]
pub struct ZipfI64 {
    items: Vec<i64>,
    cardinality: u64,
    s: f64,
    seed: u64,
}

impl ZipfI64 {
    pub fn new(size: usize, cardinality: u64, s: f64, seed: u64) -> Result<Self, SketchCoreError> {
        let dist = Zipf::new(cardinality, s)
            .map_err(|e| SketchCoreError::BadParam(format!("zipf: {e}")))?;
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let items = (0..size).map(|_| dist.sample(&mut rng) as i64).collect();
        Ok(Self {
            items,
            cardinality,
            s,
            seed,
        })
    }
}

impl Workload for ZipfI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "zipf".into(),
            size: self.items.len(),
            cardinality: Some(self.cardinality),
            zipf_s: Some(self.s),
            source_path: None,
            seed: Some(self.seed),
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

/// File-backed `i64` workload; reads little-endian int64s from a
/// binary file (matching the pre-existing `input/benchmark_data_*.bin`
/// layout).
#[derive(Debug, Clone)]
pub struct FileI64 {
    items: Vec<i64>,
    source_path: String,
}

impl FileI64 {
    pub fn load(path: &Path) -> Result<Self, SketchCoreError> {
        let bytes = std::fs::read(path).map_err(SketchCoreError::Io)?;
        if bytes.len() % 8 != 0 {
            return Err(SketchCoreError::BadParam(format!(
                "file size {} not a multiple of 8",
                bytes.len()
            )));
        }
        let items = bytes
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Ok(Self {
            items,
            source_path: path.display().to_string(),
        })
    }
}

impl Workload for FileI64 {
    type Item = i64;
    fn desc(&self) -> WorkloadDesc {
        WorkloadDesc {
            shape: "file".into(),
            size: self.items.len(),
            cardinality: None,
            zipf_s: None,
            source_path: Some(self.source_path.clone()),
            seed: None,
        }
    }
    fn items(&self) -> &[i64] {
        &self.items
    }
}

// ---------- derived workloads for string / bytes impls ----------

/// Derive a `String` workload from any `i64` workload by
/// decimal-formatting each item. Lets the existing
/// `Elastic`/`UnivMon` string sketches reuse the same
/// distributions without forking the generators.
#[derive(Debug, Clone)]
pub struct StringFromI64<W: Workload<Item = i64>> {
    items: Vec<String>,
    inner_desc: WorkloadDesc,
    _marker: std::marker::PhantomData<W>,
}

impl<W: Workload<Item = i64>> StringFromI64<W> {
    pub fn new(inner: &W) -> Self {
        let items = inner.items().iter().map(|v| v.to_string()).collect();
        Self {
            items,
            inner_desc: inner.desc(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<W: Workload<Item = i64>> Workload for StringFromI64<W> {
    type Item = String;
    fn desc(&self) -> WorkloadDesc {
        self.inner_desc.clone()
    }
    fn items(&self) -> &[String] {
        &self.items
    }
}

/// Same, but `Vec<u8>` for impls that want `&[u8]`.
#[derive(Debug, Clone)]
pub struct BytesFromI64<W: Workload<Item = i64>> {
    items: Vec<Vec<u8>>,
    inner_desc: WorkloadDesc,
    _marker: std::marker::PhantomData<W>,
}

impl<W: Workload<Item = i64>> BytesFromI64<W> {
    pub fn new(inner: &W) -> Self {
        let items = inner
            .items()
            .iter()
            .map(|v| v.to_string().into_bytes())
            .collect();
        Self {
            items,
            inner_desc: inner.desc(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<W: Workload<Item = i64>> Workload for BytesFromI64<W> {
    type Item = Vec<u8>;
    fn desc(&self) -> WorkloadDesc {
        self.inner_desc.clone()
    }
    fn items(&self) -> &[Vec<u8>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_is_reproducible_from_seed() {
        let a = UniformI64::new(100, 1000, 42);
        let b = UniformI64::new(100, 1000, 42);
        assert_eq!(a.items(), b.items());
    }

    #[test]
    fn zipf_items_in_expected_range() {
        let w = ZipfI64::new(1000, 100, 1.1, 7).unwrap();
        for v in w.items() {
            assert!(*v >= 1 && *v <= 100);
        }
    }

    #[test]
    fn string_workload_derived_length() {
        let inner = UniformI64::new(50, 100, 1);
        let s = StringFromI64::new(&inner);
        assert_eq!(s.items().len(), 50);
        assert_eq!(s.desc().shape, "uniform");
    }
}
