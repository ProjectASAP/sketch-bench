//! Synthetic data generation — see `docs/aqpbm-datagen.md`. In-memory only: no
//! files, no workspace deps. Four separable axes: distribution ([`dist`]), rule
//! ([`rule`]), rendering ([`value`]), relation ([`table`]).

pub mod column;
pub mod dist;
pub mod error;
pub mod rule;
pub mod table;
pub mod value;

pub use column::{ColumnSpec, DATA_TYPES};
pub use dist::{
    DataDistribution, Domain, NormalParameter, Sampler, UniformParameter, ZipfParameter,
};
pub use error::DataGenError;
pub use rule::{RULE_MASK_ALL, RULE_MONOTONIC_INCREASE, RULE_NONE};
pub use table::{GeneratedTable, TableDescription};
pub use value::{ColumnData, ColumnItem, StrCfg, StringOpts};

#[cfg(test)]
mod tests {
    use super::*;

    fn zipf(population_size: u64, skewness: f64, seed: u64) -> DataDistribution {
        DataDistribution::Zipf(ZipfParameter {
            skewness,
            population_size,
            seed,
        })
    }

    fn uniform(lower_bound: f64, upper_bound: f64, seed: u64) -> DataDistribution {
        DataDistribution::Uniform(UniformParameter {
            lower_bound,
            upper_bound,
            seed,
        })
    }

    fn normal(mean: f64, standard_deviation: f64, seed: u64) -> DataDistribution {
        DataDistribution::Normal(NormalParameter {
            mean,
            standard_deviation,
            seed,
        })
    }

    fn column(distribution: DataDistribution, data_type: &str) -> ColumnSpec {
        ColumnSpec {
            distribution,
            shift: None,
            cardinality: None,
            special_rule: RULE_NONE,
            data_type: data_type.into(),
            string: None,
        }
    }

    fn one(spec: ColumnSpec, row_num: u64) -> TableDescription {
        TableDescription::single("c0", spec, row_num)
    }

    // ---------- determinism and ranges ----------

    #[test]
    fn the_same_description_produces_the_same_table() {
        let d = one(column(zipf(1000, 1.1, 7), "i64"), 500);
        assert_eq!(d.generate().unwrap(), d.generate().unwrap());
    }

    /// The seed lives inside the distribution, so changing it is the one edit
    /// that redraws a column without changing what the column *is*.
    #[test]
    fn the_seed_is_what_separates_two_draws() {
        let a = one(column(zipf(1000, 1.1, 1), "i64"), 500)
            .generate()
            .unwrap();
        let b = one(column(zipf(1000, 1.1, 2), "i64"), 500)
            .generate()
            .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn zipf_ranks_stay_in_the_population() {
        let t = one(column(zipf(100, 1.2, 3), "i64"), 2000)
            .generate()
            .unwrap();
        for v in t.into_column(0).unwrap().into_i64().unwrap() {
            assert!((1..=100).contains(&v), "{v} outside 1..=100");
        }
    }

    #[test]
    fn uniform_stays_between_its_bounds() {
        let t = one(column(uniform(-5.0, 5.0, 3), "f64"), 2000)
            .generate()
            .unwrap();
        for v in t.into_column(0).unwrap().into_f64().unwrap() {
            assert!((-5.0..5.0).contains(&v), "{v} outside -5..5");
        }
    }

    /// The example the design doc gives for `shift`: a Zipf column that should
    /// live in the 15000s rather than starting at its natural rank 1.
    #[test]
    fn shift_moves_a_column_off_its_natural_base() {
        let mut spec = column(zipf(5000, 1.1, 4), "i64");
        spec.shift = Some(15000.0);
        let t = one(spec, 2000).generate().unwrap();
        for v in t.into_column(0).unwrap().into_i64().unwrap() {
            assert!((15001..=20000).contains(&v), "{v} outside 15001..=20000");
        }
    }

    #[test]
    fn shift_applies_to_normal_too() {
        let mut spec = column(normal(0.0, 1.0, 5), "f64");
        spec.shift = Some(1000.0);
        let t = one(spec, 2000).generate().unwrap();
        let values = t.into_column(0).unwrap().into_f64().unwrap();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!((mean - 1000.0).abs() < 0.2, "mean {mean} not near 1000");
    }

    // ---------- rendering ----------

    /// `data_type` picks the variant, and nothing else about the description
    /// changes — the same draws land in a different container.
    #[test]
    fn data_type_selects_the_variant() {
        for (ty, kind) in [("i64", "i64"), ("u64", "u64"), ("f64", "f64")] {
            let t = one(column(zipf(64, 1.0, 9), ty), 100).generate().unwrap();
            assert_eq!(t.data[0].kind(), kind);
        }
    }

    #[test]
    fn peeling_the_wrong_variant_names_both_types() {
        let t = one(column(zipf(64, 1.0, 9), "i64"), 10).generate().unwrap();
        let err = t
            .into_column(0)
            .unwrap()
            .into_f64()
            .unwrap_err()
            .to_string();
        assert!(err.contains("i64") && err.contains("f64"), "{err}");
    }

    #[test]
    fn an_unknown_data_type_is_refused_by_name() {
        let err = one(column(zipf(64, 1.0, 9), "i32"), 10)
            .generate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("i32"), "{err}");
    }

    #[test]
    fn a_string_column_renders_distinct_ranks_to_distinct_keys() {
        let mut spec = column(zipf(200, 0.0, 11), "string");
        spec.string = Some(StringOpts {
            alphabet: "abcdef".into(),
            min_len: 4,
            max_len: 8,
        });
        let t = one(spec, 4000).generate().unwrap();
        let values = t.into_column(0).unwrap().into_string().unwrap();
        for s in &values {
            assert!(s.len() >= 4 && s.len() <= 8, "{s} outside 4..=8");
            assert!(s.chars().all(|c| "abcdef".contains(c)), "{s}");
        }
        // Injective rendering: no more distinct keys than there are ranks.
        let mut distinct: Vec<&String> = values.iter().collect();
        distinct.sort();
        distinct.dedup();
        assert!(distinct.len() > 1 && distinct.len() <= 200);
    }

    /// Normal has no bounded domain, so there is no prefix width that can
    /// address its values injectively. Refused rather than approximated.
    #[test]
    fn a_normal_string_column_is_refused() {
        let err = one(column(normal(0.0, 1.0, 1), "string"), 10)
            .generate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("bounded domain"), "{err}");
    }

    // ---------- the special_rule mask ----------

    #[test]
    fn the_monotonic_rule_never_decreases_and_starts_at_shift() {
        let mut spec = column(uniform(0.0, 10.0, 21), "i64");
        spec.special_rule = RULE_MONOTONIC_INCREASE;
        spec.shift = Some(1_700_000_000_000.0);
        let t = one(spec, 5000).generate().unwrap();
        let values = t.into_column(0).unwrap().into_i64().unwrap();
        assert_eq!(values[0], 1_700_000_000_000);
        assert!(values.windows(2).all(|w| w[1] >= w[0]));
        assert!(values[4999] > values[0], "the series never advanced");
    }

    #[test]
    fn an_unknown_rule_bit_is_refused() {
        let mut spec = column(zipf(64, 1.0, 9), "i64");
        spec.special_rule = 0b1010;
        let err = one(spec, 10).generate().unwrap_err().to_string();
        assert!(err.contains("unknown bit"), "{err}");
    }

    #[test]
    fn a_string_column_refuses_shift_and_rules() {
        let mut spec = column(zipf(64, 1.0, 9), "string");
        spec.shift = Some(1.0);
        assert!(one(spec.clone(), 10).generate().is_err());

        spec.shift = None;
        spec.special_rule = RULE_MONOTONIC_INCREASE;
        assert!(one(spec, 10).generate().is_err());
    }

    // ---------- cardinality as a restatement ----------

    #[test]
    fn a_cardinality_that_agrees_with_the_domain_is_accepted() {
        let mut spec = column(zipf(256, 1.1, 9), "i64");
        spec.cardinality = Some(256);
        assert!(one(spec, 100).generate().is_ok());
    }

    #[test]
    fn a_cardinality_that_disagrees_names_both_numbers() {
        let mut spec = column(zipf(256, 1.1, 9), "i64");
        spec.cardinality = Some(1000);
        let err = one(spec, 100).generate().unwrap_err().to_string();
        assert!(err.contains("1000") && err.contains("256"), "{err}");
    }

    #[test]
    fn a_cardinality_over_an_unbounded_distribution_is_refused() {
        let mut spec = column(normal(0.0, 1.0, 9), "f64");
        spec.cardinality = Some(100);
        let err = one(spec, 100).generate().unwrap_err().to_string();
        assert!(err.contains("unbounded"), "{err}");
    }

    // ---------- table shape ----------

    #[test]
    fn column_num_must_match_the_column_count() {
        let mut d = one(column(zipf(64, 1.0, 9), "i64"), 10);
        d.column_num = 2;
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("column_num"), "{err}");
    }

    #[test]
    fn every_column_comes_back_the_same_length() {
        let d = TableDescription {
            column_num: 3,
            column_label: vec!["a".into(), "b".into(), "c".into()],
            column_spec: vec![
                column(zipf(64, 1.0, 1), "i64"),
                column(uniform(0.0, 8.0, 2), "f64"),
                column(zipf(16, 1.2, 3), "u64"),
            ],
            column_connected: Vec::new(),
            row_num: 777,
        };
        let t = d.generate().unwrap();
        assert_eq!(t.row_num, 777);
        for c in &t.data {
            assert_eq!(c.len(), 777);
        }
        t.validate().unwrap();
    }

    #[test]
    fn a_zero_row_table_is_refused() {
        assert!(one(column(zipf(64, 1.0, 9), "i64"), 0).generate().is_err());
    }

    // ---------- column_connected ----------

    fn flow_table(seed_b: u64, shift_b: Option<f64>) -> TableDescription {
        let mut src = column(zipf(500, 1.1, 42), "i64");
        let mut dst = column(zipf(500, 1.1, seed_b), "i64");
        src.shift = Some(10_000.0);
        dst.shift = shift_b;
        TableDescription {
            column_num: 2,
            column_label: vec!["src_ip".into(), "dst_ip".into()],
            column_spec: vec![src, dst],
            column_connected: vec![vec!["src_ip".into(), "dst_ip".into()]],
            row_num: 1000,
        }
    }

    /// Each repeated latent rank maps to one stable tuple rather than being
    /// copied into both columns.
    #[test]
    fn connected_columns_derive_a_stable_tuple() {
        let d = flow_table(42, Some(20_000.0));
        let latent = d.column_spec[0].draw(d.row_num as usize).unwrap();
        let t = d.generate().unwrap();
        let mut cols = t.into_columns();
        let dst = cols.pop().unwrap().into_i64().unwrap();
        let src = cols.pop().unwrap().into_i64().unwrap();
        assert_eq!(src.len(), 1000);

        let mut tuple_of = std::collections::HashMap::new();
        let mut repeated = 0;
        for ((draw, s), d) in latent.iter().zip(&src).zip(&dst) {
            let tuple = (*s, *d);
            let latent_rank = (*draw - 1.0) as u64;
            if let Some(previous) = tuple_of.insert(latent_rank, tuple) {
                repeated += 1;
                assert_eq!(tuple, previous, "one latent rank mapped to two tuples");
            }
        }
        assert!(repeated > 0, "the test needs repeated latent ranks");
        assert!(src.iter().zip(&dst).any(|(s, d)| d - s != 10_000));
    }

    fn mixed_bits_for_test(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }

    #[test]
    fn three_connected_columns_receive_low_to_high_chunks() {
        let distribution = zipf(2_000_000, 1.1, 42);
        let mut specs = vec![
            column(distribution.clone(), "u64"),
            column(distribution.clone(), "u64"),
            column(distribution, "u64"),
        ];
        specs[1].shift = Some(2_000_000.0);
        specs[2].shift = Some(4_000_000.0);
        let d = TableDescription {
            column_num: 3,
            column_label: vec!["a".into(), "b".into(), "c".into()],
            column_spec: specs,
            column_connected: vec![vec!["a".into(), "b".into(), "c".into()]],
            row_num: 64,
        };
        let latent = d.column_spec[0].draw(d.row_num as usize).unwrap();
        let columns = d
            .generate()
            .unwrap()
            .into_columns()
            .into_iter()
            .map(|column| column.into_u64().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(mixed_bits_for_test(0), 0xe220_a839_7b1d_cdaf);
        let width = 64 / 3;
        let mask = (1u64 << width) - 1;
        for (row, draw) in latent.iter().enumerate() {
            let bits = mixed_bits_for_test((*draw - 1.0) as u64);
            for (member, column) in columns.iter().enumerate() {
                let rank = ((bits >> (member * width)) & mask) % 2_000_000;
                let expected = 1 + rank + member as u64 * 2_000_000;
                assert_eq!(column[row], expected);
            }
        }
    }

    #[test]
    fn connected_columns_that_draw_differently_are_refused() {
        let err = flow_table(43, None).generate().unwrap_err().to_string();
        assert!(err.contains("draw differently"), "{err}");
    }

    #[test]
    fn a_connection_naming_an_unknown_column_is_refused() {
        let mut d = flow_table(42, None);
        d.column_connected = vec![vec!["src_ip".into(), "nowhere".into()]];
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("nowhere"), "{err}");
    }

    #[test]
    fn a_column_in_two_groups_is_refused() {
        let mut d = flow_table(42, None);
        d.column_connected = vec![
            vec!["src_ip".into(), "dst_ip".into()],
            vec!["src_ip".into(), "dst_ip".into()],
        ];
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("groups 0 and 1"), "{err}");
    }

    #[test]
    fn a_one_column_group_is_refused() {
        let mut d = flow_table(42, None);
        d.column_connected = vec![vec!["src_ip".into()]];
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("at least 2"), "{err}");
    }

    #[test]
    fn a_group_larger_than_the_source_word_is_refused() {
        let labels = (0..65).map(|i| format!("c{i}")).collect::<Vec<_>>();
        let spec = column(zipf(64, 1.0, 9), "i64");
        let d = TableDescription {
            column_num: 65,
            column_label: labels.clone(),
            column_spec: vec![spec; 65],
            column_connected: vec![labels],
            row_num: 10,
        };
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("at most 64"), "{err}");
    }

    #[test]
    fn a_connected_unbounded_distribution_is_refused() {
        let spec = column(normal(0.0, 1.0, 9), "f64");
        let d = TableDescription {
            column_num: 2,
            column_label: vec!["a".into(), "b".into()],
            column_spec: vec![spec.clone(), spec],
            column_connected: vec![vec!["a".into(), "b".into()]],
            row_num: 10,
        };
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("bounded integer domain"), "{err}");
    }

    #[test]
    fn a_connected_zero_sized_domain_is_refused() {
        let spec = column(uniform(0.0, 0.5, 9), "f64");
        let d = TableDescription {
            column_num: 2,
            column_label: vec!["a".into(), "b".into()],
            column_spec: vec![spec.clone(), spec],
            column_connected: vec![vec!["a".into(), "b".into()]],
            row_num: 10,
        };
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("positive integer domain"), "{err}");
    }

    #[test]
    fn duplicate_labels_are_refused() {
        let mut d = flow_table(42, None);
        d.column_label = vec!["same".into(), "same".into()];
        d.column_connected = Vec::new();
        let err = d.generate().unwrap_err().to_string();
        assert!(err.contains("used twice"), "{err}");
    }

    // ---------- serde ----------

    #[test]
    fn a_description_round_trips_through_yaml() {
        let d = flow_table(42, Some(20_000.0));
        let text = serde_yaml::to_string(&d).unwrap();
        let back: TableDescription = serde_yaml::from_str(&text).unwrap();
        assert_eq!(d, back);
        assert_eq!(d.generate().unwrap(), back.generate().unwrap());
    }

    #[test]
    fn a_description_deserializes_from_the_documented_yaml_shape() {
        let yaml = "
column_num: 1
column_label: [key]
row_num: 100
column_spec:
  - data_type: i64
    cardinality: 200
    distribution:
      kind: zipf
      skewness: 1.1
      population_size: 200
      seed: 1
";
        let d: TableDescription = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(d.column_spec[0].cardinality, Some(200));
        assert_eq!(d.column_spec[0].distribution.seed(), 1);
        assert_eq!(d.generate().unwrap().data[0].len(), 100);
    }

    #[test]
    fn a_string_column_comes_back_the_length_it_claims() {
        let t = one(column(zipf(64, 1.0, 9), "string"), 50)
            .generate()
            .unwrap();
        assert_eq!(t.data[0].len(), 50);
    }
}
