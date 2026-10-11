//! Synthetic data generation — see `docs/aqpbm-datagen.md`. In-memory only: no
//! files, no workspace deps. Four separable axes: distribution ([`dist`]), rule
//! ([`rule`]), rendering ([`value`]), relation ([`table`]).

pub mod burst;
pub mod column;
pub mod dist;
pub mod error;
pub mod rule;
pub mod table;
pub mod value;

pub use burst::{inject_interval_bursts, BurstSpec};
pub use column::{ColumnSpec, DATA_TYPES};
pub use dist::{
    DataDistribution, Domain, NormalParameter, ParetoParameter, Sampler, UniformParameter,
    ZipfParameter,
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

    fn pareto(alpha: f64, scale: f64, seed: u64) -> DataDistribution {
        DataDistribution::Pareto(ParetoParameter { alpha, scale, seed })
    }

    fn column(distribution: DataDistribution, data_type: &str) -> ColumnSpec {
        ColumnSpec {
            distribution,
            shift: None,
            cardinality: None,
            special_rule: RULE_NONE,
            data_type: data_type.into(),
            string: None,
            child_of: None,
            fan_out: None,
            scale_by: None,
            scale_range: None,
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

    /// The MLE of the shape, `n / sum(ln(x / scale))`, recovers `alpha` from a
    /// large fixed-seed sample.
    #[test]
    fn pareto_mle_recovers_alpha() {
        let (alpha, scale) = (1.5, 2.0);
        let t = one(column(pareto(alpha, scale, 13), "f64"), 200_000)
            .generate()
            .unwrap();
        let values = t.into_column(0).unwrap().into_f64().unwrap();
        assert!(values.iter().all(|&x| x >= scale));
        let log_sum: f64 = values.iter().map(|x| (x / scale).ln()).sum();
        let alpha_hat = values.len() as f64 / log_sum;
        assert!(
            (alpha_hat - alpha).abs() / alpha < 0.05,
            "alpha_hat {alpha_hat} not within 5% of {alpha}"
        );
    }

    /// An i64 column is the f64 draws floored: every draw is >= scale > 0, so
    /// the renderer's `as` truncation is a floor. The i64-only exact quantile
    /// baseline scores a Pareto stream through this.
    #[test]
    fn pareto_i64_floors_the_f64_draws() {
        let draw = |data_type| {
            one(column(pareto(1.1, 1000.0, 7), data_type), 50_000)
                .generate()
                .unwrap()
                .into_column(0)
                .unwrap()
        };
        let floats = draw("f64").into_f64().unwrap();
        let ints = draw("i64").into_i64().unwrap();
        assert!(ints.iter().all(|&v| v >= 1000));
        assert!(ints
            .iter()
            .zip(&floats)
            .all(|(&i, f)| i == f.floor() as i64));
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

    // ---------- child_of ----------

    fn child(parent: usize, fan_out: u64, distribution: DataDistribution) -> ColumnSpec {
        ColumnSpec {
            child_of: Some(parent),
            fan_out: Some(fan_out),
            ..column(distribution, "string")
        }
    }

    /// region (4) → service (fan-out 25, Zipf) → endpoint (fan-out 5), then a value.
    fn hier_table() -> TableDescription {
        TableDescription {
            column_num: 4,
            column_label: vec![
                "region".into(),
                "service".into(),
                "endpoint".into(),
                "value".into(),
            ],
            column_spec: vec![
                column(uniform(0.0, 4.0, 1), "string"),
                child(0, 25, zipf(25, 1.2, 2)),
                child(1, 5, uniform(0.0, 5.0, 3)),
                column(zipf(1000, 1.1, 4), "i64"),
            ],
            column_connected: Vec::new(),
            row_num: 20_000,
        }
    }

    /// Each child splits into its parent, a `.`, and an index below `fan_out`;
    /// returns the distinct children seen under each parent.
    fn children_by_parent(
        parents: &[String],
        children: &[String],
        fan_out: u64,
    ) -> std::collections::HashMap<String, std::collections::HashSet<String>> {
        let mut by_parent: std::collections::HashMap<_, std::collections::HashSet<_>> =
            Default::default();
        for (parent, child) in parents.iter().zip(children) {
            let (head, index) = child.rsplit_once('.').expect("a child has a `.`");
            assert_eq!(head, parent, "'{child}' does not start with its parent");
            assert!(index.parse::<u64>().unwrap() < fan_out, "'{child}'");
            by_parent
                .entry(parent.clone())
                .or_default()
                .insert(child.clone());
        }
        by_parent
    }

    #[test]
    fn a_child_table_is_deterministic() {
        let d = hier_table();
        assert_eq!(d.generate().unwrap(), d.generate().unwrap());
    }

    #[test]
    fn every_child_extends_its_parent_within_its_fan_out() {
        let t = hier_table().generate().unwrap();
        let region = t.data[0].as_string().unwrap();
        let service = t.data[1].as_string().unwrap();
        let endpoint = t.data[2].as_string().unwrap();

        let services = children_by_parent(region, service, 25);
        assert_eq!(services.len(), 4);
        // 20k draws over 25 Zipf(1.2) ranks reach every service under every region.
        assert!(services.values().all(|s| s.len() == 25), "{services:?}");

        let endpoints = children_by_parent(service, endpoint, 5);
        assert_eq!(endpoints.len(), 100);
        assert!(endpoints.values().all(|e| e.len() <= 5));
    }

    /// The child index is the column's own draw, ranked: the distribution is
    /// what picks it, and the parent plays no part.
    #[test]
    fn the_child_index_follows_the_columns_distribution() {
        let d = hier_table();
        let draws = d.column_spec[1].draw(d.row_num as usize).unwrap();
        let t = d.generate().unwrap();
        let service = t.data[1].as_string().unwrap();
        let mut counts = [0u64; 25];
        for (draw, s) in draws.iter().zip(service) {
            let index: u64 = s.rsplit_once('.').unwrap().1.parse().unwrap();
            assert_eq!(index, (draw - 1.0) as u64);
            counts[index as usize] += 1;
        }
        // Zipf(1.2) over 25 ranks gives rank 1 about 30% of the mass.
        let head = counts[0] as f64 / d.row_num as f64;
        assert!((0.25..0.35).contains(&head), "{head}");
        assert!(counts[0] > counts[1] && counts[1] > counts[24]);
    }

    #[test]
    fn hydra_hier_yaml_generates_its_hierarchy() {
        let d =
            TableDescription::from_path(std::path::Path::new("../configs/datagen/hydra_hier.yaml"))
                .unwrap();
        let t = d.generate().unwrap();
        let col = |i: usize| t.data[i].as_string().unwrap();
        assert_eq!(children_by_parent(col(0), col(1), 25).len(), 4);
        assert_eq!(children_by_parent(col(1), col(2), 25).len(), 100);
    }

    /// The schema-width cuts keep the first labels and the value column.
    #[test]
    fn hydra_hier_cuts_keep_their_leading_labels() {
        for (file, labels) in [("hydra_hier_d2.yaml", 2), ("hydra_hier_d3.yaml", 3)] {
            let path = format!("../configs/datagen/{file}");
            let d = TableDescription::from_path(std::path::Path::new(&path)).unwrap();
            assert_eq!(d.column_num, labels + 1, "{file}");
            let t = d.generate().unwrap();
            let col = |i: usize| t.data[i].as_string().unwrap();
            assert_eq!(children_by_parent(col(0), col(1), 25).len(), 4, "{file}");
        }
    }

    #[test]
    fn bad_child_columns_are_refused_by_name() {
        type Edit = fn(&mut TableDescription);
        let cases: Vec<(&str, Edit)> = vec![
            ("earlier column", |d| d.column_spec[1].child_of = Some(1)),
            ("earlier column", |d| d.column_spec[1].child_of = Some(9)),
            ("not a string column", |d| {
                d.column_spec[0].data_type = "i64".into()
            }),
            ("needs fan_out", |d| d.column_spec[1].fan_out = None),
            ("only to a child_of", |d| d.column_spec[1].child_of = None),
            ("fan_out must be > 0", |d| {
                d.column_spec[1].fan_out = Some(0)
            }),
            ("fan_out 24 disagrees", |d| {
                d.column_spec[1].fan_out = Some(24)
            }),
            ("bounded domain", |d| {
                d.column_spec[1].distribution = normal(0.0, 1.0, 2)
            }),
            ("has to be `string`", |d| {
                d.column_spec[1].data_type = "u64".into()
            }),
            ("whole numbers", |d| {
                d.column_spec[2].distribution = uniform(0.0, 5.9, 3)
            }),
            ("cardinality", |d| d.column_spec[1].cardinality = Some(25)),
            ("`string:` block", |d| {
                d.column_spec[1].string = Some(StringOpts::default())
            }),
        ];
        for (want, edit) in cases {
            let mut d = hier_table();
            edit(&mut d);
            let err = d.generate().unwrap_err().to_string();
            assert!(err.contains(want), "wanted '{want}' in: {err}");
        }
    }

    // ---------- scale_by ----------

    /// service (25 labels) and a latency scaled by it, next to the same
    /// latency unscaled.
    fn scaled_table() -> TableDescription {
        TableDescription {
            column_num: 3,
            column_label: vec!["service".into(), "plain".into(), "scaled".into()],
            column_spec: vec![
                column(zipf(25, 1.1, 1), "string"),
                column(pareto(2.0, 1000.0, 5), "f64"),
                ColumnSpec {
                    scale_by: Some(0),
                    scale_range: Some([1.0, 10.0]),
                    ..column(pareto(2.0, 1000.0, 5), "f64")
                },
            ],
            column_connected: Vec::new(),
            row_num: 20_000,
        }
    }

    /// Every row is its unscaled draw times one factor per label, the factors
    /// stay in the range and spread across it.
    #[test]
    fn each_label_scales_its_rows_by_one_factor_in_the_range() {
        let d = scaled_table();
        let t = d.generate().unwrap();
        assert_eq!(t, d.generate().unwrap());
        let service = t.data[0].as_string().unwrap();
        let plain = t.data[1].as_f64().unwrap();
        let scaled = t.data[2].as_f64().unwrap();
        let mut factors: std::collections::HashMap<&str, f64> = Default::default();
        for ((s, p), v) in service.iter().zip(plain).zip(scaled) {
            let f = *factors
                .entry(s)
                .or_insert_with(|| d.column_spec[2].scale_factor(s));
            assert_eq!(*v, p * f, "'{s}'");
        }
        assert_eq!(factors.len(), 25);
        let (min, max) = factors
            .values()
            .fold((f64::MAX, f64::MIN), |(a, b), &f| (a.min(f), b.max(f)));
        assert!(min >= 1.0 && max <= 10.0, "{min}..{max}");
        // 25 log-uniform factors over a decade span most of it.
        assert!(min < 2.0 && max > 5.0, "{min}..{max}");
    }

    /// The factor depends on the label and the seed only.
    #[test]
    fn the_scale_factor_is_fixed_by_label_and_seed() {
        let spec = scaled_table().column_spec[2].clone();
        assert_eq!(spec.scale_factor("abcd"), spec.scale_factor("abcd"));
        assert_ne!(spec.scale_factor("abcd"), spec.scale_factor("abce"));
        let reseeded = ColumnSpec {
            distribution: pareto(2.0, 1000.0, 6),
            ..spec.clone()
        };
        assert_ne!(spec.scale_factor("abcd"), reseeded.scale_factor("abcd"));
        let flat = ColumnSpec {
            scale_range: Some([3.0, 3.0]),
            ..spec
        };
        assert_eq!(flat.scale_factor("abcd"), 3.0);
    }

    #[test]
    fn bad_scaled_columns_are_refused_by_name() {
        type Edit = fn(&mut TableDescription);
        let cases: Vec<(&str, Edit)> = vec![
            ("earlier column", |d| d.column_spec[2].scale_by = Some(2)),
            ("earlier column", |d| d.column_spec[2].scale_by = Some(9)),
            ("'plain', which is not a string", |d| {
                d.column_spec[2].scale_by = Some(1)
            }),
            ("needs scale_range", |d| d.column_spec[2].scale_range = None),
            ("only to a scale_by", |d| d.column_spec[2].scale_by = None),
            ("has to be `f64`, not 'i64'", |d| {
                d.column_spec[2].data_type = "i64".into()
            }),
            ("has to be `f64`, not 'u64'", |d| {
                d.column_spec[2].data_type = "u64".into()
            }),
            ("0 < lo <= hi", |d| {
                d.column_spec[2].scale_range = Some([0.0, 10.0])
            }),
            ("0 < lo <= hi", |d| {
                d.column_spec[2].scale_range = Some([5.0, 2.0])
            }),
        ];
        for (want, edit) in cases {
            let mut d = scaled_table();
            edit(&mut d);
            let err = d.generate().unwrap_err().to_string();
            assert!(err.contains(want), "wanted '{want}' in: {err}");
        }
    }

    /// The eval's latency spec scales by service and leaves the labels alone.
    #[test]
    fn hydra_http_latency_scales_by_service() {
        let path = std::path::Path::new("../configs/datagen/hydra_http_latency.yaml");
        let d = TableDescription::from_path(path).unwrap();
        assert_eq!(d.column_label[1], "service");
        assert_eq!(d.column_spec[4].scale_by, Some(1));
        let mut plain = d.clone();
        plain.column_spec[4].scale_by = None;
        plain.column_spec[4].scale_range = None;
        plain.row_num = 1000;
        let mut scaled = d;
        scaled.row_num = 1000;
        let (a, b) = (plain.generate().unwrap(), scaled.generate().unwrap());
        assert_eq!(a.data[..4], b.data[..4]);
        assert_ne!(a.data[4], b.data[4]);
    }

    // ---------- serde ----------

    #[test]
    fn a_description_round_trips_through_yaml() {
        let d = flow_table(42, Some(20_000.0));
        let text = serde_norway::to_string(&d).unwrap();
        let back: TableDescription = serde_norway::from_str(&text).unwrap();
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
        let d: TableDescription = serde_norway::from_str(yaml).unwrap();
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
