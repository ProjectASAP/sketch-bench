use crate::params::*;
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{require_positive, BuildError};

pub mod oxide;
pub mod sketchlib;

pub type Record<K> = (K, i64);

pub trait UnivMonKey: FrequencyValue + std::fmt::Debug {
    type Bytes<'a>: AsRef<[u8]>
    where
        Self: 'a;

    fn key_bytes(&self) -> Self::Bytes<'_>;
}

impl UnivMonKey for i64 {
    type Bytes<'a> = [u8; 8];

    #[inline(always)]
    fn key_bytes(&self) -> [u8; 8] {
        self.to_le_bytes()
    }
}

impl UnivMonKey for u64 {
    type Bytes<'a> = [u8; 8];

    #[inline(always)]
    fn key_bytes(&self) -> [u8; 8] {
        self.to_le_bytes()
    }
}

impl UnivMonKey for f64 {
    type Bytes<'a> = [u8; 8];

    #[inline(always)]
    fn key_bytes(&self) -> [u8; 8] {
        self.to_bits().to_le_bytes()
    }
}

impl UnivMonKey for String {
    type Bytes<'a> = &'a [u8];

    #[inline(always)]
    fn key_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

fn check_shape(p: &UnivMonParams, algorithm: &str) -> Result<(), BuildError> {
    for (name, value) in [
        ("heap_size", p.heap_size),
        ("sketch_row", p.sketch_row),
        ("sketch_col", p.sketch_col),
        ("layer_size", p.layer_size),
    ] {
        require_positive(algorithm, name, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::oxide::*;
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    fn small() -> ParamSet {
        ParamSet::of(&UnivMonParams {
            heap_size: 1000,
            sketch_row: 3,
            sketch_col: 1024,
            layer_size: 4,
        })
    }

    fn stream() -> Vec<Record<u64>> {
        vec![(7, 5), (11, 3), (7, 2), (13, 9), (11, 1)]
    }

    fn stream_of_strings() -> Vec<Record<String>> {
        stream()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    #[test]
    fn canonical_params_build() {
        let canonical = ParamSet::of(&UnivMonParams::canonical());
        assert!(build_univmon_lib(&canonical).is_ok());
        assert!(build_univmon_oxide(&canonical).is_ok());
    }

    #[test]
    fn zero_dimensions_are_refused_by_name() {
        let bad = ParamSet::of(&UnivMonParams {
            heap_size: 1000,
            sketch_row: 3,
            sketch_col: 0,
            layer_size: 4,
        });
        let Err(err) = build_univmon_lib(&bad) else {
            panic!("a zero dimension must be refused, not built");
        };
        assert!(
            err.to_string().contains("sketch_col"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn oxide_refuses_a_heap_size_it_cannot_build_at() {
        let bad = ParamSet::of(&UnivMonParams {
            heap_size: 32,
            sketch_row: 5,
            sketch_col: 2048,
            layer_size: 8,
        });
        let Err(err) = build_univmon_oxide(&bad) else {
            panic!("a heap size the library fixes must be refused, not silently ignored");
        };
        let err = err.to_string();
        assert!(
            err.contains("heap_size"),
            "error should name the field: {err}"
        );
        assert!(build_univmon_lib(&bad).is_ok());
    }

    #[test]
    fn lib_footprint_is_the_layers_times_a_layer() {
        let sketch = build_univmon_lib(&small()).expect("canonical dimensions build");
        let counters = (3 * 1024 + 3) * std::mem::size_of::<i64>();
        let heap = 1000 * std::mem::size_of::<asap_sketchlib::input::HHItem>();
        assert_eq!(memory_univmon_lib(&sketch), 4 * (counters + heap));
    }

    #[test]
    fn the_l1_norm_is_the_weight_the_stream_carried() {
        let total: i64 = stream().iter().map(|(_, value)| value).sum();
        let mut lib = build_univmon_lib(&small()).expect("canonical dimensions build");
        let mut oxide = build_univmon_oxide(&small()).expect("canonical dimensions build");
        for record in stream().iter() {
            lib.feed(record);
            oxide.feed(record);
        }
        assert_eq!(oxide.estimate_l1_norm(), total as f64);
        let estimate = lib.estimate_l1_norm();
        assert!(
            (estimate - total as f64).abs() < 0.05 * total as f64,
            "the stream carried {total}, estimated {estimate}"
        );
    }

    #[test]
    fn the_cardinality_is_the_keys_the_stream_carried() {
        let mut lib = build_univmon_lib(&small()).expect("canonical dimensions build");
        for record in stream().iter() {
            lib.feed(record);
        }
        let estimate = lib.estimate_cardinality();
        assert!(
            (estimate - 3.0).abs() < 0.5,
            "three distinct keys, estimated {estimate}"
        );
    }

    #[test]
    fn every_key_width_carries_the_same_weight() {
        let total: i64 = stream().iter().map(|(_, value)| value).sum();
        let mut lib = build_univmon_lib(&small()).expect("canonical dimensions build");
        let mut oxide = build_univmon_oxide(&small()).expect("canonical dimensions build");
        for record in stream_of_strings().iter() {
            lib.feed(record);
            oxide.feed(record);
        }
        assert_eq!(oxide.estimate_l1_norm(), total as f64);
        let estimate = lib.estimate_l1_norm();
        assert!(
            (estimate - total as f64).abs() < 0.05 * total as f64,
            "the stream carried {total}, estimated {estimate}"
        );
        let keys = lib.estimate_cardinality();
        assert!(
            (keys - 3.0).abs() < 0.5,
            "three distinct keys, estimated {keys}"
        );
    }

    #[test]
    fn a_float_key_is_read_by_its_bits() {
        let bits: Vec<u8> = 1.0f64.key_bytes().to_vec();
        assert_eq!(bits, 1.0f64.to_bits().to_le_bytes().to_vec());
        assert_ne!(0.0f64.key_bytes(), (-0.0f64).key_bytes());
    }

    #[test]
    fn merging_shards_carries_both_halves() {
        let (mut left, mut right) = (
            build_univmon_lib(&small()).expect("canonical dimensions build"),
            build_univmon_lib(&small()).expect("canonical dimensions build"),
        );
        for record in stream().iter() {
            left.feed(record);
            right.feed(record);
        }
        let alone = left.estimate_l1_norm();
        left.inner.merge(&right.inner);
        let folded = left.estimate_l1_norm();
        assert!(
            (folded - 2.0 * alone).abs() < 0.05 * 2.0 * alone,
            "two identical shards carry twice one, {alone} folded to {folded}"
        );
    }
}
